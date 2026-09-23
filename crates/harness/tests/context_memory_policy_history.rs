// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use fixture::*;
use serde_json::Value;
use sts2_harness::context_memory::policy_owner::*;

/// Construct authenticated malformed internal-store fixtures with synthetic key/data.
/// This deliberately bypasses owner publication, so recovery must still validate typed bounds.
fn malformed_envelope(original: &[u8], mutate: impl FnOnce(&mut Value)) -> Vec<u8> {
    let aad = serde_json::to_vec(&(
        "ascension.context-memory.policy-store.v1",
        scope(),
        "journal",
        1_u64,
    ))
    .unwrap();
    let cipher = XChaCha20Poly1305::new((&[7_u8; 32]).into());
    let plaintext = cipher
        .decrypt(
            &XNonce::try_from(&original[..24]).unwrap(),
            Payload {
                msg: &original[24..],
                aad: &aad,
            },
        )
        .unwrap();
    let mut value: Value = serde_json::from_slice(&plaintext).unwrap();
    mutate(&mut value);
    let plaintext = serde_json::to_vec(&value).unwrap();
    assert!(plaintext.len() <= MAX_POLICY_JOURNAL_BYTES);
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut nonce).unwrap();
    let ciphertext = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .unwrap();
    let mut envelope = nonce.to_vec();
    envelope.extend_from_slice(&ciphertext);
    envelope
}

fn assert_recovery_rejects_without_claim(mutate: impl FnOnce(&mut Value)) {
    let fixture = Fixture::new();
    fixture.adopt();
    let connection = rusqlite::Connection::open(&fixture.path).unwrap();
    let (epoch, original): (i64, Vec<u8>) = connection
        .query_row("SELECT epoch,envelope FROM policy_journal", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    let malformed = malformed_envelope(&original, mutate);
    connection
        .execute("UPDATE policy_journal SET envelope=?1", [&malformed])
        .unwrap();
    assert!(matches!(
        MemoryPolicyOwner::open(
            &fixture.path,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly
        ),
        Err(PolicyOwnerError::Corrupt)
    ));
    let after: i64 = connection
        .query_row("SELECT epoch FROM policy_journal", [], |row| row.get(0))
        .unwrap();
    assert_eq!(after, epoch);
    connection
        .execute("UPDATE policy_journal SET envelope=?1", [&original])
        .unwrap();
    // The healthy owner remains usable after restoring the original authenticated fixture.
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
}

#[test]
fn authenticated_over_cardinality_history_never_claims_the_store() {
    for (field, limit) in [
        ("policies", MAX_POLICY_VERSIONS),
        ("reviews", MAX_POLICY_REVIEWS),
        ("approvals", MAX_POLICY_REVIEWS),
        ("adoptions", MAX_POLICY_REVIEWS),
        ("receipts", MAX_POLICY_RECEIPTS),
    ] {
        assert_recovery_rejects_without_claim(|journal| {
            let original = journal[field][0].clone();
            journal[field] = Value::Array(vec![original; limit + 1]);
        });
    }
}

#[test]
fn authenticated_over_raw_policy_history_never_claims_the_store() {
    assert_recovery_rejects_without_claim(|journal| {
        let raw = journal["policies"][0]["raw"].as_array_mut().unwrap();
        raw.resize(MAX_POLICY_BYTES + 1, Value::from(32));
    });
}
