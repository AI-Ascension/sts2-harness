// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used)]

use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use sts2_harness as harness_api;
use sts2_harness::exo_lifecycle::{
    AuthorityVector, EXO_LIFECYCLE_WIRE_V2, ExoLifecycleRuntimeTransport, InvocationManifest,
    JournalConfig, LifecycleAuthorityPort, LifecycleError, LifecycleOwner, LifecycleProcessEffect,
    MAX_INPUT_BYTES, MAX_WIRE_ID_BYTES,
};
use sts2_harness::provider_session::{
    NativeCapabilities, ProviderSessionBroker, ProviderSessionMode, ProviderSessionPolicy,
};
use sts2_harness::{
    ExecutionCancellation, ExecutionStore, ExoDecisionRequest, ExoProcessConfig, ExoTransport,
    encode_bridge_request, parse_bridge_request, sha256_hex,
};

#[path = "support/exo_lifecycle.rs"]
mod fixture;

type Factory =
    Box<dyn FnMut(&str, &str, &ExoDecisionRequest) -> Result<InvocationManifest, LifecycleError>>;
type Transport = ExoLifecycleRuntimeTransport<Factory>;

#[test]
fn exact_one_shot_duplicate_reuses_completed_receipt_and_rejects_changed_request()
-> Result<(), String> {
    let mut fixture = fixture::Fixture::new();
    let candidate = Rc::new(RefCell::new(pending_manifest(&fixture.manifest)));
    let (mut transport, _, log) = configured_transport(&mut fixture, candidate.clone(), false)?;
    let first = transport
        .exchange(&fixture.input, 8 * 1024, 1_000)
        .map_err(|error| format!("{error:?}"))?;
    let replay = transport
        .exchange(&fixture.input, 8 * 1024, 1_000)
        .map_err(|error| format!("{error:?}"))?;
    if first != replay || effect_count(&log)? != 1 {
        return Err(String::from(
            "exact duplicate did not return the stored response after one effect",
        ));
    }

    let mut changed_input = fixture.input.clone();
    changed_input.push(b' ');
    if transport.exchange(&changed_input, 8 * 1024, 1_000).is_ok() {
        return Err(String::from(
            "completed result was reused for changed exact input bytes",
        ));
    }
    let mut changed = candidate.borrow().clone();
    changed.authority.catalog_digest = sha256_hex("changed-current-catalog");
    *candidate.borrow_mut() = changed;
    if transport.exchange(&fixture.input, 8 * 1024, 1_000).is_ok() {
        return Err(String::from(
            "completed result was reused under changed current authority",
        ));
    }
    if effect_count(&log)? != 1 {
        return Err(String::from(
            "changed duplicate path issued another provider effect",
        ));
    }
    Ok(())
}

#[test]
fn unknown_one_shot_duplicate_stays_held_without_a_retry() -> Result<(), String> {
    let mut fixture = fixture::Fixture::new();
    let candidate = Rc::new(RefCell::new(pending_manifest(&fixture.manifest)));
    let (mut transport, store, log) = configured_transport(&mut fixture, candidate, true)?;
    if transport.exchange(&fixture.input, 8 * 1024, 1_000).is_ok() {
        return Err(String::from(
            "malformed native response unexpectedly completed",
        ));
    }
    if !store
        .borrow()
        .decision(&fixture.manifest.execution_id)
        .map_err(|error| error.to_string())?
        .unknown
    {
        return Err(String::from("ambiguous result was not retained as unknown"));
    }
    if transport.exchange(&fixture.input, 8 * 1024, 1_000).is_ok() || effect_count(&log)? != 1 {
        return Err(String::from(
            "unknown duplicate retried the provider effect",
        ));
    }
    Ok(())
}

/// The published wire width has to survive composition, not just validation. `tests_identity_width`
/// proves the manifest predicate admits 512 bytes; this drives a maximum-width identity through the
/// real one-shot path — owner mint, broker admission, dispatch and receipt — because that is where
/// the hidden ceiling used to appear and where the request was admitted and then never settled.
#[test]
fn a_maximum_width_identity_dispatches_through_the_one_shot_path() -> Result<(), String> {
    let mut fixture = fixture::Fixture::new();
    let mut request = parse_bridge_request(
        include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/request.json"),
        MAX_INPUT_BYTES,
    )
    .map_err(|error| error.to_string())?;
    request.model_execution_id = "e".repeat(MAX_WIRE_ID_BYTES);
    let input = encode_bridge_request("request-1", "turn-1", &request, MAX_INPUT_BYTES)
        .map_err(|error| error.to_string())?;
    let mut candidate = pending_manifest(&fixture.manifest);
    candidate.execution_id = request.model_execution_id.clone();
    let candidate = Rc::new(RefCell::new(candidate));
    let (mut transport, store, log) = configured_transport(&mut fixture, candidate, false)?;
    let response = transport
        .exchange(&input, 8 * 1024, 1_000)
        .map_err(|error| format!("{error:?}"))?;
    if response.is_empty() || effect_count(&log)? != 1 {
        return Err(String::from(
            "a maximum-width identity did not dispatch exactly one provider effect",
        ));
    }
    let decision = store
        .borrow()
        .decision(&request.model_execution_id)
        .map_err(|error| error.to_string())?;
    if decision.unknown || !decision.completed {
        return Err(String::from(
            "a maximum-width identity was admitted without settling",
        ));
    }
    Ok(())
}

fn configured_transport(
    fixture: &mut fixture::Fixture,
    candidate: Rc<RefCell<InvocationManifest>>,
    malformed_receipt: bool,
) -> Result<(Transport, Rc<RefCell<ExecutionStore>>, std::path::PathBuf), String> {
    let log = fixture.root.join("one-shot-effect-count");
    let script = fixture.root.join("one-shot-bridge");
    let request_id = if malformed_receipt {
        "wrong-request"
    } else {
        "request-1"
    };
    let response = format!(
        "{{\"wire_version\":\"sts2.exo-bridge-wire-v2\",\"request_id\":\"{request_id}\",\"turn_id\":\"turn-1\",\"outcome\":\"decision\",\"decision\":{{\"decision\":\"action\",\"action_id\":\"combat.end-turn\",\"rationale\":\"bounded\",\"confidence\":90}},\"error_code\":null,\"native\":{{\"agent_id\":\"agent-1\",\"conversation_id\":\"conversation-1\",\"session_id\":\"session-1\",\"turn_id\":\"native-turn-1\",\"event_cursor\":\"event-1\"}}}}"
    );
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho x >> '{}'\ncat >/dev/null\nprintf '%s' '{}'\n",
            log.display(),
            response
        ),
    )
    .map_err(|error| error.to_string())?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let effect = LifecycleProcessEffect::new(
        ExoProcessConfig::new(
            script.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                String::from("/tmp/one-shot-config"),
                String::from("digest"),
            ],
            None,
            Vec::new(),
        )
        .map_err(|error| error.to_string())?,
        ExecutionCancellation::default(),
        8 * 1024,
        1_000,
        3,
    )
    .map_err(|error| format!("{error:?}"))?;

    let scope = fixture.manifest.scope.clone();
    let executor = sha256_hex("reviewed-one-shot-executor");
    let configuration = sha256_hex("reviewed-one-shot-configuration");
    let profile = sha256_hex(
        serde_json::to_vec(&serde_json::json!({
            "adapter":"sts2-exo-lifecycle-v2",
            "configuration_sha256":configuration,
            "executor_sha256":executor,
            "wire":EXO_LIFECYCLE_WIRE_V2
        }))
        .map_err(|error| error.to_string())?,
    );
    let capabilities = NativeCapabilities::reviewed_exo_lifecycle(
        "sts2-exo-lifecycle-v2",
        profile.clone(),
        executor,
        sha256_hex(EXO_LIFECYCLE_WIRE_V2),
    )
    .map_err(|error| error.to_string())?;
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = String::from("fixture-realm");
    policy.profile_sha256 = profile.clone();
    let owner_token = String::from("owner-fixture");
    let broker =
        ProviderSessionBroker::new(scope.clone(), policy, capabilities, owner_token.clone())
            .map_err(|error| error.to_string())?;
    let journal = JournalConfig {
        directory: fixture.root.join("one-shot-journal"),
        legacy_path: None,
        store_id: String::from("one-shot-journal"),
        scope,
        owner_binding_digest: sha256_hex(b"authenticated-owner-fixture"),
    };
    let owner = LifecycleOwner::create(
        journal,
        [7; 32],
        broker,
        owner_token,
        Arc::clone(&fixture.authority) as Arc<dyn LifecycleAuthorityPort>,
    )
    .map_err(|error| format!("{error:?}"))?;
    let replacement = ExecutionStore::open_in_memory().map_err(|error| error.to_string())?;
    let store = Rc::new(RefCell::new(std::mem::replace(
        &mut fixture.store,
        replacement,
    )));
    let manifest_factory: Factory =
        Box::new(move |_: &str, _: &str, _: &ExoDecisionRequest| Ok(candidate.borrow().clone()));
    let transport = ExoLifecycleRuntimeTransport::new(
        owner,
        store.clone(),
        fixture.fingerprint.clone(),
        effect,
        3_600,
        Arc::new(SystemTime::now),
        manifest_factory,
    )
    .map_err(|error| format!("{error:?}"))?;
    Ok((transport, store, log))
}

fn pending_manifest(source: &InvocationManifest) -> InvocationManifest {
    let mut manifest = source.clone();
    manifest.provider_attempt_id = String::from("pending-provider-attempt");
    manifest.reservation_id = String::from("pending-reservation");
    manifest.binding_id = String::from("pending-binding");
    manifest.operation_id = String::from("pending-operation");
    manifest.prepared_id = String::from("pending-prepared");
    manifest.package_digest = sha256_hex("reviewed-one-shot-executor");
    manifest.config_digest = sha256_hex("reviewed-one-shot-configuration");
    manifest.profile_digest = sha256_hex(
        serde_json::to_vec(&serde_json::json!({
            "adapter":"sts2-exo-lifecycle-v2",
            "configuration_sha256":sha256_hex("reviewed-one-shot-configuration"),
            "executor_sha256":sha256_hex("reviewed-one-shot-executor"),
            "wire":EXO_LIFECYCLE_WIRE_V2
        }))
        .expect("profile"),
    );
    manifest.authority = AuthorityVector {
        owner_epoch: 1,
        auth_epoch: 1,
        session_epoch: 1,
        history_epoch: 0,
        compaction_epoch: 0,
        revocation_epoch: 0,
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        state_id: source.authority.state_id.clone(),
        generation: source.authority.generation,
        catalog_digest: source.authority.catalog_digest.clone(),
    };
    manifest
}

fn effect_count(path: &std::path::Path) -> Result<usize, String> {
    std::fs::read_to_string(path)
        .map(|value| value.lines().count())
        .map_err(|error| error.to_string())
}
