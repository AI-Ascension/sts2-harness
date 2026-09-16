// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used)]

use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;

use sts2_harness as harness_api;
use sts2_harness::exo_lifecycle::{
    ExoLifecycleRuntimeTransport, InvocationManifest, LifecycleError, LifecycleProcessEffect,
};
use sts2_harness::{
    ExecutionCancellation, ExecutionFingerprint, ExecutionStore, ExoDecisionRequest,
    ExoProcessConfig, ExoTransport,
};

#[path = "support/exo_lifecycle.rs"]
mod fixture;

#[test]
fn v2_receipt_is_durable_and_a_replay_uses_the_stored_result()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture::Fixture::new();
    let log = fixture.root.join("spawned");
    let script = fixture.root.join("bridge");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho x >> '{}'\ncat >/dev/null\nprintf '%s' '{}'\n",
            log.display(),
            r#"{"wire_version":"sts2.exo-bridge-wire-v2","request_id":"request-1","turn_id":"turn-1","outcome":"decision","decision":{"decision":"action","action_id":"combat.end-turn","rationale":"bounded","confidence":90},"error_code":null,"native":{"agent_id":"agent-1","conversation_id":"conversation-1","session_id":"session-1","turn_id":"turn-1","event_cursor":"event-1"}}"#
        ),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let process = ExoProcessConfig::new(
        script.to_string_lossy(),
        vec![
            String::from("--run-v2"),
            String::from("/tmp/exo-lifecycle-config"),
            String::from("digest"),
        ],
        None,
        Vec::new(),
    )?;
    let effect = LifecycleProcessEffect::new(
        process,
        ExecutionCancellation::default(),
        8 * 1024,
        1_000,
        3,
    )?;
    let replacement = ExecutionStore::open_in_memory()?;
    let store = Rc::new(RefCell::new(std::mem::replace(
        &mut fixture.store,
        replacement,
    )));
    let manifest = fixture.manifest.clone();
    let mut transport = ExoLifecycleRuntimeTransport::new(
        fixture.owner(),
        store.clone(),
        fixture.fingerprint.clone(),
        effect,
        move |_: &str,
              _: &str,
              _: &ExoDecisionRequest|
              -> Result<InvocationManifest, LifecycleError> { Ok(manifest.clone()) },
    );
    let first = transport
        .exchange(&fixture.input, 8 * 1024, 1_000)
        .map_err(|error| format!("{error:?}"))?;
    assert!(!first.is_empty());
    assert_eq!(std::fs::read_to_string(&log)?.lines().count(), 1);
    assert!(
        store
            .borrow()
            .decision(&fixture.manifest.execution_id)?
            .completed
    );
    let replay = transport
        .exchange(&fixture.input, 8 * 1024, 1_000)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(first, replay);
    assert_eq!(std::fs::read_to_string(&log)?.lines().count(), 1);
    Ok(())
}

#[test]
fn malformed_native_receipt_becomes_unknown_and_is_not_replayed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture::Fixture::new();
    let log = fixture.root.join("spawned");
    let script = fixture.root.join("bridge");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho x >> '{}'\ncat >/dev/null\nprintf '%s' '{}'\n",
            log.display(),
            r#"{"wire_version":"sts2.exo-bridge-wire-v2","request_id":"wrong","turn_id":"turn-1","outcome":"decision","decision":{"decision":"action","action_id":"combat.end-turn","rationale":"bounded","confidence":90},"error_code":null,"native":{"agent_id":"agent-1","conversation_id":"conversation-1","session_id":"session-1","turn_id":"turn-1","event_cursor":"event-1"}}"#
        ),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let effect = LifecycleProcessEffect::new(
        ExoProcessConfig::new(
            script.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                String::from("/tmp/exo-lifecycle-config"),
                String::from("digest"),
            ],
            None,
            Vec::new(),
        )?,
        ExecutionCancellation::default(),
        8 * 1024,
        1_000,
        3,
    )?;
    let replacement = ExecutionStore::open_in_memory()?;
    let store = Rc::new(RefCell::new(std::mem::replace(
        &mut fixture.store,
        replacement,
    )));
    let manifest = fixture.manifest.clone();
    let mut transport = ExoLifecycleRuntimeTransport::new(
        fixture.owner(),
        store.clone(),
        fixture.fingerprint.clone(),
        effect,
        move |_: &str,
              _: &str,
              _: &ExoDecisionRequest|
              -> Result<InvocationManifest, LifecycleError> { Ok(manifest.clone()) },
    );
    assert!(transport.exchange(&fixture.input, 8 * 1024, 1_000).is_err());
    assert!(
        store
            .borrow()
            .decision(&fixture.manifest.execution_id)?
            .unknown
    );
    assert_eq!(std::fs::read_to_string(&log)?.lines().count(), 1);
    assert!(transport.exchange(&fixture.input, 8 * 1024, 1_000).is_err());
    assert_eq!(std::fs::read_to_string(&log)?.lines().count(), 1);
    Ok(())
}

#[test]
fn cancellation_kills_the_process_path_and_holds_the_decision_unknown()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture::Fixture::new();
    let log = fixture.root.join("spawned");
    let script = fixture.root.join("bridge");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho x >> '{}'\ncat >/dev/null\nsleep 5\n",
            log.display()
        ),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let cancellation = ExecutionCancellation::default();
    let canceller = cancellation.clone();
    let effect = LifecycleProcessEffect::new(
        ExoProcessConfig::new(
            script.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                String::from("/tmp/exo-lifecycle-config"),
                String::from("digest"),
            ],
            None,
            Vec::new(),
        )?,
        cancellation,
        8 * 1024,
        5_000,
        3,
    )?;
    let replacement = ExecutionStore::open_in_memory()?;
    let store = Rc::new(RefCell::new(std::mem::replace(
        &mut fixture.store,
        replacement,
    )));
    let manifest = fixture.manifest.clone();
    let mut transport = ExoLifecycleRuntimeTransport::new(
        fixture.owner(),
        store.clone(),
        fixture.fingerprint.clone(),
        effect,
        move |_: &str,
              _: &str,
              _: &ExoDecisionRequest|
              -> Result<InvocationManifest, LifecycleError> { Ok(manifest.clone()) },
    );
    let cancel = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        canceller.cancel();
    });
    assert!(transport.exchange(&fixture.input, 8 * 1024, 5_000).is_err());
    cancel.join().map_err(|_| "canceller panicked")?;
    assert!(
        store
            .borrow()
            .decision(&fixture.manifest.execution_id)?
            .unknown
    );
    assert_eq!(std::fs::read_to_string(&log)?.lines().count(), 1);
    Ok(())
}

#[test]
fn failed_durable_admission_starts_no_process() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture::Fixture::new();
    let log = fixture.root.join("spawned");
    let script = fixture.root.join("bridge");
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho x >> '{}'\n", log.display()),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let effect = LifecycleProcessEffect::new(
        ExoProcessConfig::new(
            script.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                String::from("/tmp/exo-lifecycle-config"),
                String::from("digest"),
            ],
            None,
            Vec::new(),
        )?,
        ExecutionCancellation::default(),
        8 * 1024,
        1_000,
        3,
    )?;
    let replacement = ExecutionStore::open_in_memory()?;
    let store = Rc::new(RefCell::new(std::mem::replace(
        &mut fixture.store,
        replacement,
    )));
    let manifest = fixture.manifest.clone();
    let wrong = ExecutionFingerprint::new("other", "build", "state", "config", "provider")?;
    let mut transport = ExoLifecycleRuntimeTransport::new(
        fixture.owner(),
        store,
        wrong,
        effect,
        move |_: &str,
              _: &str,
              _: &ExoDecisionRequest|
              -> Result<InvocationManifest, LifecycleError> { Ok(manifest.clone()) },
    );
    assert!(transport.exchange(&fixture.input, 8 * 1024, 1_000).is_err());
    assert!(!log.exists());
    Ok(())
}
