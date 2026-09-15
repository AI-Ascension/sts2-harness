// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use crate::exo_lifecycle::{InvocationManifest, LifecycleEntry, LifecyclePhase, NativeIdentity};

fn entry(scope: &SessionScope, index: usize) -> LifecycleEntry {
    let digest = crate::sha256_hex("fixture");
    let manifest: InvocationManifest = serde_json::from_value(serde_json::json!({
        "scope": scope, "execution_id": format!("execution-{index}"), "episode_attempt_id": "attempt",
        "trajectory_id": "trajectory", "provider_attempt_id": format!("provider-{index}"),
        "reservation_id": format!("reservation-{index}"), "binding_id": "binding",
        "operation_id": format!("operation-{index}"), "prepared_id": "prepared",
        "request_id": "request", "host_turn_id": "turn", "input_digest": digest, "input_length": 131072,
        "config_digest": digest, "package_digest": digest, "profile_digest": digest,
        "model_revision": "revision", "reserved_units": 1,
        "authority": { "owner_epoch": 1, "auth_epoch": 1, "session_epoch": 1, "history_epoch": 0,
            "compaction_epoch": 0, "revocation_epoch": 0, "lease_id": "lease", "lease_epoch": 1,
            "state_id": "state", "generation": 0, "catalog_digest": digest }
    })).expect("manifest");
    let mut entry = LifecycleEntry::prepared(manifest, 1);
    entry.phase = LifecyclePhase::Unknown;
    entry.possible_write = true;
    entry.permit_revision = Some(1);
    entry
}

#[test]
fn entry_count_keeps_unknown_and_terminal_records_and_rejects_129() {
    let mut fixture = Fixture::new();
    fixture.snapshot.broker.policy.max_completed_turns = 128;
    fixture.snapshot.entries = (0..128)
        .map(|index| entry(&fixture.config.scope, index))
        .collect();
    fixture.snapshot.entries[1].phase = LifecyclePhase::Fenced;
    let mut binding = SessionBinding::candidate(
        "binding",
        fixture.config.scope.clone(),
        "branch",
        SessionPurpose::Executable,
        "realm",
        crate::sha256_hex("fixture"),
        "2099-01-01T00:00:00Z",
    )
    .expect("binding");
    binding.state = BindingState::Recovering;
    fixture.snapshot.broker.bindings.push(binding);
    fixture.snapshot.broker.operations = fixture
        .snapshot
        .entries
        .iter()
        .map(|entry| NativeOperation {
            schema: SESSION_OPERATION_SCHEMA.into(),
            operation_id: entry.manifest.operation_id.clone(),
            scope: fixture.config.scope.clone(),
            binding_id: "binding".into(),
            kind: NativeOperationKind::Turn,
            idempotency_key: entry.manifest.execution_id.clone(),
            request_sha256: entry.manifest.operation_digest().expect("digest"),
            state: NativeOperationState::Unknown,
            owner_epoch: 1,
            session_epoch: 1,
            generation_permission: true,
            generation_class: true,
            automatic_retry: false,
            auto_resume: false,
            game_effects: 0,
            terminal_evidence_ref: None,
        })
        .collect();
    assert!(fixture.snapshot.validate(&fixture.config).is_ok());
    fixture
        .snapshot
        .entries
        .push(entry(&fixture.config.scope, 128));
    assert_eq!(
        fixture.snapshot.validate(&fixture.config),
        Err(LifecycleError::Corrupt)
    );
}

#[test]
fn exact_id_cursor_and_input_bounds_are_enforced() {
    let fixture = Fixture::new();
    let mut entry = entry(&fixture.config.scope, 0);
    entry.manifest.execution_id = "e".repeat(128);
    entry.native = Some(NativeIdentity {
        agent_id: "agent".into(),
        conversation_id: "conversation".into(),
        session_id: "session".into(),
        turn_id: "turn".into(),
        event_cursor: "c".repeat(128),
    });
    assert!(entry.validate().is_ok());
    entry.manifest.input_length += 1;
    assert!(entry.validate().is_err());
    entry.manifest.input_length -= 1;
    entry.manifest.execution_id.push('e');
    assert!(entry.validate().is_err());
    entry.manifest.execution_id.pop();
    entry
        .native
        .as_mut()
        .expect("native")
        .event_cursor
        .push('c');
    assert!(entry.validate().is_err());
}

#[test]
fn aggregate_plaintext_limit_is_exact_and_ciphertext_is_bounded() {
    let mut fixture = Fixture::new();
    // Direct serializer boundary fixture; not a qualified native capability profile.
    let base = serde_json::to_vec(&fixture.snapshot)
        .expect("snapshot")
        .len();
    fixture
        .snapshot
        .broker
        .policy
        .credential_realm_ref
        .push_str(&"x".repeat(MAX_HISTORY_BYTES - base));
    assert_eq!(
        serde_json::to_vec(&fixture.snapshot)
            .expect("snapshot")
            .len(),
        MAX_HISTORY_BYTES
    );
    let encoded = io::encode(&fixture.config, &[7; 32], &fixture.snapshot).expect("exact bound");
    assert_eq!(encoded.len(), super::super::types::MAX_ENVELOPE);
    fixture
        .snapshot
        .broker
        .policy
        .credential_realm_ref
        .push('x');
    assert_eq!(
        io::encode(&fixture.config, &[7; 32], &fixture.snapshot),
        Err(LifecycleError::Capacity)
    );
}

#[test]
fn authenticated_unknown_duplicate_version_and_truncated_payloads_are_rejected() {
    let fixture = Fixture::new();
    let plain = serde_json::to_string(&fixture.snapshot).expect("snapshot");
    let invalid = [
        plain.replacen('{', "{\"unknown\":true,", 1),
        plain.replacen('{', "{\"revision\":1,", 1),
        plain.replace("owner-journal.v2", "owner-journal.v3"),
    ];
    for value in invalid {
        let encoded =
            io::seal_test_plaintext(&fixture.config, &[7; 32], value.as_bytes()).expect("seal");
        assert!(io::decode(&fixture.config, &[7; 32], &encoded).is_err());
    }
    let mut encoded = io::encode(&fixture.config, &[7; 32], &fixture.snapshot).expect("encode");
    encoded.pop();
    assert!(io::decode(&fixture.config, &[7; 32], &encoded).is_err());
    encoded[0] ^= 1;
    assert!(io::decode(&fixture.config, &[7; 32], &encoded).is_err());
}

#[test]
fn revision_overflow_never_wraps_or_mutates_disk() {
    let mut fixture = Fixture::new();
    fixture.snapshot.revision = u64::MAX;
    let mut journal =
        OwnerJournal::create(fixture.config.clone(), [7; 32], &fixture.snapshot).expect("create");
    let before = io::read(&journal.lease).expect("read");
    assert_eq!(
        journal.commit(&fixture.snapshot),
        Err(LifecycleError::Capacity)
    );
    assert_eq!(io::read(&journal.lease).expect("read"), before);
    assert_eq!(journal.check(), Err(LifecycleError::Poisoned));
}
