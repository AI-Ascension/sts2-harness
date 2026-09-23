// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::panic)]

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sts2_harness as harness_api;
use sts2_harness::exo_lifecycle::*;
use sts2_harness::*;

#[path = "support/exo_lifecycle.rs"]
mod fixture;
use fixture::{Effect, Fixture, Handle};

struct CountedAuthority {
    inner: Arc<fixture::Authority>,
    claims: AtomicUsize,
}
impl LifecycleAuthorityPort for CountedAuthority {
    fn claim<'a>(
        &'a self,
        request: &OwnerClaim<'_>,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.claims.fetch_add(1, Ordering::SeqCst);
        self.inner.claim(request)
    }
    fn admit<'a>(
        &'a self,
        manifest: &InvocationManifest,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.inner.admit(manifest)
    }
    fn consume<'a>(
        &'a self,
        manifest: &InvocationManifest,
        digest: &str,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.inner.consume(manifest, digest)
    }
}

const MAGIC: &[u8] = b"ASCENSION-PROVIDER-METADATA-ENC2\0";

fn aad(f: &Fixture) -> Vec<u8> {
    let mut aad = b"ascension.provider-session.owner-journal.v2\0".to_vec();
    for value in [
        sha256_hex(serde_json::to_vec(&f.config.scope).expect("scope")),
        f.config.store_id.clone(),
    ] {
        aad.extend_from_slice(&(value.len() as u64).to_be_bytes());
        aad.extend_from_slice(value.as_bytes());
    }
    aad
}

fn decode(f: &Fixture, bytes: &[u8]) -> Value {
    let cipher = XChaCha20Poly1305::new(&Key::from([7_u8; 32]));
    let end = MAGIC.len() + 24;
    let plaintext = cipher
        .decrypt(
            &XNonce::try_from(&bytes[MAGIC.len()..end]).expect("24-byte nonce"),
            Payload {
                msg: &bytes[end..],
                aad: &aad(f),
            },
        )
        .expect("synthetic authenticated journal");
    serde_json::from_slice(&plaintext).expect("snapshot")
}

fn encode(f: &Fixture, value: &Value) -> Vec<u8> {
    // New random nonce for each synthetic authenticated-corruption fixture.
    let mut nonce = [0; 24];
    getrandom::fill(&mut nonce).expect("fixture nonce");
    let cipher = XChaCha20Poly1305::new(&Key::from([7_u8; 32]));
    let ciphertext = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &serde_json::to_vec(value).expect("snapshot"),
                aad: &aad(f),
            },
        )
        .expect("seal fixture");
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&ciphertext);
    bytes
}

fn corrupt(value: &mut Value, case: &str) {
    let operation_id = value["entries"][0]["manifest"]["operation_id"].clone();
    let index = value["broker"]["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .position(|operation| operation["operation_id"] == operation_id)
        .expect("turn");
    match case {
        "missing-operation" => {
            value["entries"][0]["manifest"]["operation_id"] = "missing-operation".into()
        }
        "missing-binding" => {
            value["entries"][0]["manifest"]["binding_id"] = "missing-binding".into()
        }
        "wrong-kind" => value["broker"]["operations"][index]["kind"] = "refresh".into(),
        "wrong-binding" => {
            let mut other = value["broker"]["bindings"][0].clone();
            other["binding_id"] = "other-binding".into();
            other["state"] = "held".into();
            other["game_dispatch_capability"] = false.into();
            value["broker"]["bindings"]
                .as_array_mut()
                .expect("bindings")
                .push(other);
            value["broker"]["operations"][index]["binding_id"] = "other-binding".into();
        }
        "wrong-digest" => {
            value["broker"]["operations"][index]["request_sha256"] =
                sha256_hex("wrong-request").into()
        }
        "wrong-epoch" => value["broker"]["operations"][index]["owner_epoch"] = 2.into(),
        "wrong-profile" => {
            value["entries"][0]["manifest"]["profile_digest"] = sha256_hex("wrong-profile").into()
        }
        "wrong-phase" => {
            value["entries"][0]["phase"] = "admitted".into();
            value["entries"][0]["possible_write"] = false.into();
            value["entries"][0]["permit_revision"] = Value::Null;
        }
        "unused-phase" => {
            value["entries"][0]["phase"] = "failed_before_send".into();
            value["entries"][0]["possible_write"] = false.into();
            value["entries"][0]["permit_revision"] = Value::Null;
        }
        _ => panic!("unknown fixture case"),
    }
}

#[test]
fn semantic_corruption_rejects_before_authority_claim_and_preserves_ciphertext() {
    for case in [
        "missing-operation",
        "missing-binding",
        "wrong-kind",
        "wrong-binding",
        "wrong-digest",
        "wrong-epoch",
        "wrong-profile",
        "wrong-phase",
        "unused-phase",
    ] {
        let mut f = Fixture::new();
        let mut owner = f.owner();
        let mut effect = Effect::default();
        owner
            .start(
                f.manifest.clone(),
                &f.input,
                &mut f.store,
                &f.fingerprint,
                &mut effect,
            )
            .expect("start");
        drop(owner);
        let path = f.config.directory.join("journal.enc");
        let original = std::fs::read(&path).expect("journal");
        let mut value = decode(&f, &original);
        let epoch = value["claim_epoch"].as_u64().expect("epoch");
        corrupt(&mut value, case);
        let corrupt = encode(&f, &value);
        std::fs::write(&path, &corrupt).expect("synthetic corruption");
        let decision = f
            .store
            .decision(&f.manifest.execution_id)
            .expect("decision");
        let charge = f
            .store
            .provider_reservation(&f.manifest.reservation_id)
            .expect("charge");
        let authority = Arc::new(CountedAuthority {
            inner: f.authority.clone(),
            claims: AtomicUsize::new(0),
        });
        assert!(
            LifecycleOwner::open(
                f.config.clone(),
                [7; 32],
                "restart-owner".into(),
                &f.policy,
                &f.capabilities,
                authority.clone(),
            )
            .is_err(),
            "{case}"
        );
        assert_eq!(authority.claims.load(Ordering::SeqCst), 0, "{case}");
        assert_eq!(
            std::fs::read(&path).expect("retained evidence"),
            corrupt,
            "{case}"
        );
        assert_eq!(
            f.store
                .decision(&f.manifest.execution_id)
                .expect("decision"),
            decision
        );
        assert_eq!(
            f.store
                .provider_reservation(&f.manifest.reservation_id)
                .expect("charge"),
            charge
        );
        std::fs::write(&path, &original).expect("restore original fixture");
        let reopened = LifecycleOwner::open(
            f.config.clone(),
            [7; 32],
            "restart-owner".into(),
            &f.policy,
            &f.capabilities,
            authority.clone(),
        )
        .expect("valid restart");
        assert_eq!(reopened.claim_epoch(), epoch + 1);
        assert_eq!(authority.claims.load(Ordering::SeqCst), 1);
        assert_eq!(effect.calls, 1);
    }
}

#[test]
fn repeated_restart_preserves_unknown_and_completed_store_repair() {
    let mut f = Fixture::new();
    let mut owner = f.owner();
    let mut effect = Effect::default();
    owner
        .start(
            f.manifest.clone(),
            &f.input,
            &mut f.store,
            &f.fingerprint,
            &mut effect,
        )
        .expect("start");
    let result = Handle {
        ready: true,
        units: Some(3),
    }
    .poll()
    .expect("poll")
    .expect("result");
    f.store
        .complete_provider_with_result(
            &f.manifest.reservation_id,
            &result.result_ref,
            &sha256_hex(&result.response),
            &result.response,
            3,
        )
        .expect("completed store");
    drop(owner);
    for expected_epoch in [2, 3] {
        let reopened = f.reopen().expect("held restart");
        assert_eq!(reopened.claim_epoch(), expected_epoch);
        assert_eq!(reopened.entries()[0].phase, LifecyclePhase::Unknown);
    }
    let mut repaired = f.reopen().expect("restart");
    repaired
        .reconcile_stored(&f.manifest, &f.input, &f.store)
        .expect("repair");
    drop(repaired);
    let mut reopened = f.reopen().expect("restart repaired metadata");
    assert_eq!(reopened.entries()[0].phase, LifecyclePhase::Completed);
    assert!(
        reopened
            .start(
                f.manifest.clone(),
                &f.input,
                &mut f.store,
                &f.fingerprint,
                &mut effect
            )
            .is_err()
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn completed_history_survives_later_binding_epochs_and_repeated_restart() {
    let mut f = Fixture::new();
    let mut owner = f.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(mut handle) = owner
        .start(
            f.manifest.clone(),
            &f.input,
            &mut f.store,
            &f.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    owner.poll(&mut handle, &mut f.store).expect("complete");
    drop(owner);
    let path = f.config.directory.join("journal.enc");
    let mut value = decode(&f, &std::fs::read(&path).expect("journal"));
    // Historical completed operation epochs remain immutable while later binding work advances.
    let binding = &mut value["broker"]["bindings"][0];
    for field in ["session_epoch", "history_epoch", "compaction_epoch"] {
        binding[field] = (binding[field].as_u64().expect("epoch") + 1).into();
    }
    std::fs::write(&path, encode(&f, &value)).expect("later binding fixture");
    for expected_epoch in [2, 3] {
        let reopened = f.reopen().expect("completed historical restart");
        assert_eq!(reopened.claim_epoch(), expected_epoch);
        assert_eq!(reopened.entries()[0].phase, LifecyclePhase::Completed);
    }
    assert_eq!(effect.calls, 1);
}
