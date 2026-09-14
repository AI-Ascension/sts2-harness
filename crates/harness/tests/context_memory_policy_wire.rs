// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use fixture::*;
use serde_json::{Value, json};
use sts2_harness::context_memory::{policy_owner::*, *};

fn stored(fixture: &Fixture) -> (i64, Vec<u8>) {
    rusqlite::Connection::open(&fixture.path)
        .unwrap()
        .query_row("SELECT epoch, envelope FROM policy_journal", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
}

fn wire_schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../contracts/context-memory/policy.schema.json"
    ))
    .unwrap()
}

fn observed_invalid_originals() -> Vec<Value> {
    let original = serde_json::to_value(policy(2, 9000)).unwrap();
    let mut cases = Vec::new();
    for revision in [Value::Null, json!("not a revision")] {
        let mut value = original.clone();
        value["phase2_revision_id"] = revision;
        cases.push(value);
    }
    let mut missing = original.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("phase2_revision_id");
    cases.push(missing);
    for generation in [0, 9_007_199_254_740_992_u64] {
        let mut value = original.clone();
        value["corpus_generation"] = json!(generation);
        cases.push(value);
    }
    cases
}

#[test]
fn wire_invalid_imports_never_consume_a_version_or_receipt() {
    let fixture = Fixture::new();
    let retained = fixture.import();
    let before = stored(&fixture);
    let schema = wire_schema();
    let oracle = jsonschema::validator_for(&schema).unwrap();
    let original = serde_json::to_value(policy(2, 9000)).unwrap();
    let mut cases = observed_invalid_originals();
    let mut draft_without_revision = original.clone();
    draft_without_revision["status"] = json!("draft");
    draft_without_revision
        .as_object_mut()
        .unwrap()
        .remove("phase2_revision_id");
    cases.push(draft_without_revision);
    for field in schema["required"].as_array().unwrap() {
        let mut value = original.clone();
        value
            .as_object_mut()
            .unwrap()
            .remove(field.as_str().unwrap());
        cases.push(value);
    }
    for field in schema["properties"]["scope"]["required"]
        .as_array()
        .unwrap()
    {
        let mut value = original.clone();
        value["scope"]
            .as_object_mut()
            .unwrap()
            .remove(field.as_str().unwrap());
        cases.push(value);
    }
    for revision in [json!(""), json!("a".repeat(129))] {
        let mut value = original.clone();
        value["phase2_revision_id"] = revision;
        cases.push(value);
    }
    for (index, value) in cases.into_iter().enumerate() {
        assert!(
            !oracle.is_valid(&value),
            "case {index} must be wire invalid"
        );
        let raw = serde_json::to_vec(&value).unwrap();
        let key = format!("invalid-{index}");
        assert_eq!(
            fixture.owner.execute(
                access(),
                PolicyCommand::Import {
                    key: key.clone(),
                    raw: raw.clone()
                }
            ),
            Err(PolicyOwnerError::SchemaInvalid),
            "case {index}"
        );
        assert_eq!(
            fixture.owner.lookup_receipt(access(), &key),
            Err(PolicyOwnerError::Missing)
        );
        assert!(matches!(
            fixture.owner.inspect_policy(
                access(),
                &SavedPolicyRef {
                    policy_id: "saved-policy".to_owned(),
                    version: 2,
                    raw_sha256: sha256_hex(&raw),
                }
            ),
            Err(PolicyOwnerError::Missing)
        ));
        assert_eq!(stored(&fixture), before, "case {index} mutated storage");
    }
    assert_eq!(
        fixture
            .owner
            .inspect_policy(access(), &reference(&retained))
            .unwrap()
            .raw_bytes(),
        retained
    );
}

#[test]
fn schema_valid_draft_null_and_safe_integer_edges_retain_exact_over_profile_bytes() {
    let mut draft = policy(1, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES);
    draft.status = PolicyStatus::Draft;
    draft.phase2_revision_id = None;
    let mut maximum = policy(
        9_007_199_254_740_991,
        MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES,
    );
    maximum.corpus_generation = 9_007_199_254_740_991;
    let oracle = jsonschema::validator_for(&wire_schema()).unwrap();
    for (index, value) in [draft, maximum].into_iter().enumerate() {
        let fixture = Fixture::new();
        let raw = bytes(&value);
        assert!(oracle.is_valid(&serde_json::from_slice(&raw).unwrap()));
        fixture
            .owner
            .execute(
                access(),
                PolicyCommand::Import {
                    key: format!("valid-{index}"),
                    raw: raw.clone(),
                },
            )
            .unwrap();
        let reopened = MemoryPolicyOwner::open(
            &fixture.path,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly,
        )
        .unwrap();
        assert_eq!(
            reopened
                .inspect_policy(access(), &reference(&raw))
                .unwrap()
                .raw_bytes(),
            raw
        );
    }
}

#[test]
fn safe_integer_maximum_imports_but_one_over_and_float_encodings_do_not() {
    let fixture = Fixture::new();
    let mut maximum = policy(9_007_199_254_740_991, 9000);
    maximum.corpus_generation = 9_007_199_254_740_991;
    let raw = bytes(&maximum);
    fixture
        .owner
        .execute(
            access(),
            PolicyCommand::Import {
                key: "maximum".to_owned(),
                raw: raw.clone(),
            },
        )
        .unwrap();
    for field in ["version", "corpus_generation"] {
        let mut value = serde_json::to_value(policy(2, 9000)).unwrap();
        value[field] = json!(9_007_199_254_740_992_u64);
        assert_eq!(
            fixture.owner.execute(
                access(),
                PolicyCommand::Import {
                    key: format!("over-{field}"),
                    raw: serde_json::to_vec(&value).unwrap(),
                }
            ),
            Err(PolicyOwnerError::SchemaInvalid)
        );
    }
    for (index, number) in ["2.0", "2e0", "2.5"].into_iter().enumerate() {
        let text = String::from_utf8(bytes(&policy(2, 9000)))
            .unwrap()
            .replace("\"version\": 2", &format!("\"version\": {number}"));
        let oracle = jsonschema::validator_for(&wire_schema()).unwrap();
        assert_eq!(
            oracle.is_valid(&serde_json::from_str(&text).unwrap()),
            index != 2
        );
        assert_eq!(
            fixture.owner.execute(
                access(),
                PolicyCommand::Import {
                    key: format!("numeric-{index}"),
                    raw: text.into_bytes(),
                }
            ),
            Err(if index == 2 {
                PolicyOwnerError::SchemaInvalid
            } else {
                PolicyOwnerError::UnsupportedNumericRepresentation
            })
        );
    }
    let reopened = MemoryPolicyOwner::open(
        &fixture.path,
        [7; 32],
        fixture.authority.clone(),
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    assert_eq!(
        reopened
            .inspect_policy(access(), &reference(&raw))
            .unwrap()
            .raw_bytes(),
        raw
    );
}

#[test]
fn authenticated_wire_invalid_originals_cannot_claim_on_reopen() {
    // Synthetic encrypted records model a store admitted by the older importer. No private input.
    let fixture = Fixture::new();
    let retained = fixture.import();
    let before = stored(&fixture);
    let aad = serde_json::to_vec(&(
        "ascension.context-memory.policy-store.v1",
        scope(),
        "journal",
        1_u64,
    ))
    .unwrap();
    let cipher = XChaCha20Poly1305::new((&[7_u8; 32]).into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&before.1[..24]),
            Payload {
                msg: &before.1[24..],
                aad: &aad,
            },
        )
        .unwrap();
    let original: Value = serde_json::from_slice(&plain).unwrap();
    let connection = rusqlite::Connection::open(&fixture.path).unwrap();
    for value in observed_invalid_originals() {
        let raw = serde_json::to_vec(&value).unwrap();
        let mut journal = original.clone();
        journal["policies"][0]["raw"] = json!(raw);
        journal["policies"][0]["reference"]["version"] = json!(2);
        journal["policies"][0]["reference"]["raw_sha256"] = json!(sha256_hex(&raw));
        let typed: MemoryPolicy = serde_json::from_slice(&raw).unwrap();
        journal["policies"][0]["execution_sha256"] =
            json!(sha256_hex(serde_json::to_vec(&typed).unwrap()));
        let mut nonce = [0_u8; 24];
        getrandom::fill(&mut nonce).unwrap();
        let mut envelope = nonce.to_vec();
        envelope.extend(
            cipher
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &serde_json::to_vec(&journal).unwrap(),
                        aad: &aad,
                    },
                )
                .unwrap(),
        );
        connection
            .execute("UPDATE policy_journal SET envelope=?1", [&envelope])
            .unwrap();
        assert!(matches!(
            MemoryPolicyOwner::open(
                &fixture.path,
                [7; 32],
                fixture.authority.clone(),
                PolicyStoreConsent::SyntheticOnly,
            ),
            Err(PolicyOwnerError::SchemaInvalid)
        ));
        assert_eq!(stored(&fixture), (before.0, envelope));
    }
    connection
        .execute("UPDATE policy_journal SET envelope=?1", [&before.1])
        .unwrap();
    assert_eq!(
        fixture
            .owner
            .inspect_policy(access(), &reference(&retained))
            .unwrap()
            .raw_bytes(),
        retained
    );
}
