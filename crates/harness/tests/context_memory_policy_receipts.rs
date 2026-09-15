// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
#[path = "support/memory_policy_receipts.rs"]
mod receipts;
use fixture::*;
use receipts::*;
use serde_json::{Value, json};
use sts2_harness::context_memory::{policy_owner::*, *};

#[test]
fn every_operation_requires_a_matching_result_and_request_fingerprint() {
    let history = History::new();
    for (index, other_result) in [
        (0, "saved-policy:2"),
        (1, "review-two"),
        (2, "policy-approval-2"),
        (3, "policy-binding-2"),
        (4, "review-one"),
    ] {
        history.reject(index, |journal| {
            journal["receipts"][index]["result_id"] = json!("policy-binding-nonexistent");
        });
        history.reject(index, |journal| {
            journal["receipts"][index]["result_id"] = json!(other_result);
        });
        history.reject(index, |journal| {
            journal["receipts"][index]["request_sha256"] = json!(sha256_hex(b"another command"));
        });
    }
    history.reject(1, |journal| {
        journal["receipts"][1]["operation"] = json!("propose_revalidation");
    });
}

#[test]
fn canonical_ids_and_stored_subject_lineage_are_required() {
    let history = History::new();
    history.reject(3, |journal| {
        journal["receipts"][3]["operation_id"] = json!("policy-operation-999");
    });
    history.reject(6, |journal| {
        journal["receipts"][6]["operation_id"] = journal["receipts"][0]["operation_id"].clone();
    });
    for binding_id in ["binding-alias", "policy-binding-1"] {
        history.reject(6, |journal| {
            journal["adoptions"][1]["binding_id"] = json!(binding_id);
            journal["active"]["binding_id"] = json!(binding_id);
            journal["receipts"][6]["result_id"] = json!(binding_id);
        });
    }
    history.reject(5, |journal| {
        journal["approvals"][1]["approval_id"] = json!("approval-alias");
        journal["adoptions"][1]["approval_id"] = json!("approval-alias");
        journal["active"]["approval_id"] = json!("approval-alias");
        journal["receipts"][5]["result_id"] = json!("approval-alias");
    });
    for index in [1, 2, 3] {
        history.reject(index, |journal| {
            journal["receipts"][index]["subject"] = json!("other");
        });
    }
}

#[test]
fn creation_receipts_cannot_be_removed_duplicated_or_reordered() {
    let history = History::new();
    for index in [1, 2, 6] {
        history.reject(index, |journal| {
            journal["receipts"].as_array_mut().unwrap().remove(index);
            renumber(journal);
        });
    }
    for index in [1, 2, 3] {
        let mut repeated = history.commands[index].clone();
        rekey(&mut repeated, "duplicate-creation");
        history.reject(index, |journal| {
            let mut receipt = journal["receipts"][index].clone();
            receipt["idempotency_key"] = json!(key(&repeated));
            receipt["request_sha256"] = json!(fingerprint(&repeated));
            journal["receipts"].as_array_mut().unwrap().push(receipt);
            renumber(journal);
        });
    }
    history.reject(2, |journal| {
        journal["receipts"].as_array_mut().unwrap().swap(1, 2);
        renumber(journal);
    });
}

#[test]
fn unreceipted_policy_review_and_approval_records_are_corrupt() {
    for orphan in 0..3 {
        let mut history = History::new();
        if orphan == 0 {
            history.execute(PolicyCommand::Import {
                key: "orphan-policy".to_owned(),
                raw: bytes(&policy(3, MAX_OPTIONAL_BYTES)),
            });
        } else {
            let source = reference(&bytes(&policy(2, MAX_OPTIONAL_BYTES)));
            history.execute(PolicyCommand::ProposeRevalidation {
                key: "orphan-review".to_owned(),
                review_id: "review-three".to_owned(),
                source,
                target_raw: bytes(&policy(2, MAX_OPTIONAL_BYTES)),
                expected_active_version: 2,
            });
            if orphan == 2 {
                let review = history
                    .fixture
                    .owner
                    .inspect_review(access(), "review-three")
                    .unwrap();
                history.execute(PolicyCommand::Approve {
                    key: "orphan-approval".to_owned(),
                    review_id: review.review_id,
                    review_sha256: review.review_sha256,
                });
            }
        }
        history.reject(history.commands.len() - 1, |journal| {
            journal["receipts"].as_array_mut().unwrap().pop();
        });
    }
}

#[test]
fn migration_history_requires_a_strictly_newer_target() {
    let history = History::new();
    history.reject(1, |journal| {
        let mut review: PolicyReview =
            serde_json::from_value(journal["reviews"][0].clone()).unwrap();
        review.source = review.target.clone();
        review.review_sha256.clear();
        review.review_sha256 = sha256_hex(serde_json::to_vec(&review).unwrap());
        journal["reviews"][0] = serde_json::to_value(&review).unwrap();
        journal["approvals"][0]["review_sha256"] = json!(review.review_sha256);
        // Keep all available command hashes consistent, isolating the strict migration relation.
        let mut commands = history.commands[..4].to_vec();
        if let PolicyCommand::ProposeMigration { source, .. } = &mut commands[1] {
            *source = review.source;
        }
        for command in &mut commands[2..4] {
            match command {
                PolicyCommand::Approve { review_sha256, .. }
                | PolicyCommand::Adopt { review_sha256, .. } => {
                    *review_sha256 = review.review_sha256.clone()
                }
                _ => unreachable!(),
            }
        }
        for (index, command) in commands.iter().enumerate().skip(1) {
            journal["receipts"][index]["request_sha256"] = Value::from(fingerprint(command));
        }
    });
}
