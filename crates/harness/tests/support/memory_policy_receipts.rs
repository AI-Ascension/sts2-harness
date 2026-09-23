// SPDX-License-Identifier: MIT
// Synthetic actual-owner histories and authenticated corruption fixtures; no private input.

#![allow(dead_code)]

use crate::fixture::*;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use serde_json::Value;
use sts2_harness::context_memory::{policy_owner::*, *};

pub struct History {
    pub fixture: Fixture,
    pub commands: Vec<PolicyCommand>,
}

impl History {
    pub fn pending_second() -> Self {
        let mut history = Self {
            fixture: Fixture::new(),
            commands: Vec::new(),
        };
        let source = bytes(&policy(1, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES));
        history.execute(PolicyCommand::Import {
            key: "original".to_owned(),
            raw: source.clone(),
        });
        history.execute(PolicyCommand::ProposeMigration {
            key: "migration".to_owned(),
            review_id: "review-one".to_owned(),
            source: reference(&source),
            target_raw: bytes(&policy(2, MAX_OPTIONAL_BYTES)),
            expected_active_version: None,
        });
        let first = history
            .fixture
            .owner
            .inspect_review(access(), "review-one")
            .unwrap();
        history.execute(PolicyCommand::Approve {
            key: "approval-one".to_owned(),
            review_id: first.review_id.clone(),
            review_sha256: first.review_sha256.clone(),
        });
        history.execute(PolicyCommand::Adopt {
            key: "adoption-one".to_owned(),
            review_id: first.review_id,
            review_sha256: first.review_sha256,
        });
        history.execute(PolicyCommand::ProposeRevalidation {
            key: "revalidation".to_owned(),
            review_id: "review-two".to_owned(),
            source: first.target,
            target_raw: bytes(&policy(2, MAX_OPTIONAL_BYTES)),
            expected_active_version: 1,
        });
        let second = history
            .fixture
            .owner
            .inspect_review(access(), "review-two")
            .unwrap();
        history.execute(PolicyCommand::Approve {
            key: "approval-two".to_owned(),
            review_id: second.review_id,
            review_sha256: second.review_sha256,
        });
        history
    }

    pub fn new() -> Self {
        let mut history = Self::pending_second();
        history.execute(history.second_adoption());
        history
    }

    pub fn second_adoption(&self) -> PolicyCommand {
        let second = self
            .fixture
            .owner
            .inspect_review(access(), "review-two")
            .unwrap();
        PolicyCommand::Adopt {
            key: "adoption-two".to_owned(),
            review_id: second.review_id,
            review_sha256: second.review_sha256,
        }
    }

    pub fn execute(&mut self, command: PolicyCommand) {
        self.fixture
            .owner
            .execute(access(), command.clone())
            .unwrap();
        self.commands.push(command);
    }

    pub fn reject(&self, command_index: usize, mutate: impl FnOnce(&mut Value)) {
        let before = row(&self.fixture);
        let malformed = malformed_envelope(&before.1, mutate);
        let connection = rusqlite::Connection::open(&self.fixture.path).unwrap();
        connection
            .execute("UPDATE policy_journal SET envelope=?1", [&malformed])
            .unwrap();
        let command = &self.commands[command_index];
        assert_eq!(
            self.fixture.owner.lookup_receipt(access(), key(command)),
            Err(PolicyOwnerError::Corrupt)
        );
        assert_eq!(row(&self.fixture), (before.0, malformed.clone()));
        assert_eq!(
            self.fixture.owner.execute(access(), command.clone()),
            Err(PolicyOwnerError::Corrupt)
        );
        assert_eq!(row(&self.fixture), (before.0, malformed.clone()));
        assert!(matches!(
            MemoryPolicyOwner::open(
                &self.fixture.path,
                [7; 32],
                self.fixture.authority.clone(),
                PolicyStoreConsent::SyntheticOnly,
            ),
            Err(PolicyOwnerError::Corrupt)
        ));
        assert_eq!(row(&self.fixture), (before.0, malformed));
        connection
            .execute("UPDATE policy_journal SET envelope=?1", [&before.1])
            .unwrap();
        // A rejected replacement did not fence the original owner or damage actual selection.
        self.fixture
            .owner
            .prepare_active(access(), preparation())
            .unwrap();
    }
}

pub fn row(fixture: &Fixture) -> (i64, Vec<u8>) {
    rusqlite::Connection::open(&fixture.path)
        .unwrap()
        .query_row("SELECT epoch,envelope FROM policy_journal", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
}

pub fn key(command: &PolicyCommand) -> &str {
    match command {
        PolicyCommand::Import { key, .. }
        | PolicyCommand::ProposeMigration { key, .. }
        | PolicyCommand::ProposeRevalidation { key, .. }
        | PolicyCommand::Approve { key, .. }
        | PolicyCommand::Adopt { key, .. } => key,
    }
}

pub fn rekey(command: &mut PolicyCommand, replacement: &str) {
    match command {
        PolicyCommand::Import { key, .. }
        | PolicyCommand::ProposeMigration { key, .. }
        | PolicyCommand::ProposeRevalidation { key, .. }
        | PolicyCommand::Approve { key, .. }
        | PolicyCommand::Adopt { key, .. } => *key = replacement.to_owned(),
    }
}

pub fn fingerprint(command: &PolicyCommand) -> String {
    sha256_hex(serde_json::to_vec(command).unwrap())
}

pub fn renumber(journal: &mut Value) {
    for (index, receipt) in journal["receipts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        receipt["sequence"] = Value::from(index as u64 + 1);
        receipt["operation_id"] = Value::from(format!("policy-operation-{}", index + 1));
    }
}

fn malformed_envelope(original: &[u8], mutate: impl FnOnce(&mut Value)) -> Vec<u8> {
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
            &XNonce::try_from(&original[..24]).unwrap(),
            Payload {
                msg: &original[24..],
                aad: &aad,
            },
        )
        .unwrap();
    let mut journal: Value = serde_json::from_slice(&plain).unwrap();
    mutate(&mut journal);
    let plaintext = serde_json::to_vec(&journal).unwrap();
    assert!(plaintext.len() <= MAX_POLICY_JOURNAL_BYTES);
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut nonce).unwrap();
    let mut envelope = nonce.to_vec();
    envelope.extend(
        cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .unwrap(),
    );
    envelope
}
