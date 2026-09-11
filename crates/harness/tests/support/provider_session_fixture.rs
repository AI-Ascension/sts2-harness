// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::provider_session::*;

#[path = "provider_session.rs"]
mod expiry_support;

pub fn scope() -> SessionScope {
    SessionScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope")
}

pub fn broker() -> ProviderSessionBroker {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sha256_hex("codex-app-server-fixture-v1");
    ProviderSessionBroker::new(
        scope,
        policy,
        NativeCapabilities::fixture(),
        "owner-fixture",
    )
    .expect("broker")
}

pub fn held_binding(broker: &mut ProviderSessionBroker) -> SessionBinding {
    let operation = broker
        .create_candidate(
            "owner-fixture",
            "create-prepared",
            "branch-prepared",
            SessionPurpose::Executable,
            expiry(),
        )
        .expect("candidate");
    broker
        .complete_candidate(
            "owner-fixture",
            &operation.operation_id,
            "native-thread-prepared",
        )
        .expect("complete candidate")
}

pub fn prepare(
    broker: &mut ProviderSessionBroker,
    binding_id: &str,
    name: &str,
) -> PreparedSessionTurn {
    prepare_with(broker, binding_id, name, Vec::new())
}

pub fn prepare_with(
    broker: &mut ProviderSessionBroker,
    binding_id: &str,
    name: &str,
    dependencies: Vec<String>,
) -> PreparedSessionTurn {
    broker
        .prepare_turn(
            "owner-fixture",
            binding_id,
            name,
            "preview-1",
            "revision-1",
            "selection-1",
            "boundary-1",
            format!("exact-suffix-{name}").into_bytes(),
            br#"{"type":"object"}"#.to_vec(),
            b"protected".to_vec(),
            dependencies,
            expiry(),
        )
        .expect("prepared")
}

pub fn item(sequence: u64) -> HistoryItem {
    HistoryItem {
        item_ref: format!("item-{sequence}"),
        turn_ref: format!("turn-{sequence}"),
        sequence,
        kind: HistoryItemKind::UserInput,
        content_ref: None,
        redacted: true,
    }
}

pub fn expiry() -> &'static str {
    expiry_support::expiry()
}

pub fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    sts2_harness::sha256_hex(value)
}
