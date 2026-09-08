// SPDX-License-Identifier: MIT

//! Quarantine closes admission without disabling authenticated stop or history.

use super::tests::{admitted_reservation, request, runtime};
use super::*;
use crate::worker_handoff::{AcknowledgmentStatus, LookupReply, WorkerCapability, WorkerRequest};
use crate::{WorkerControlMode, WorkerOwnerProof};
use serde_json::{Map, Value, json};

fn command(
    capability: WorkerCapability,
    mode: Option<&str>,
) -> Result<AuthenticatedWorkerRequest, Box<dyn std::error::Error>> {
    let fixture: &[u8] = match capability {
        WorkerCapability::Probe => include_bytes!(
            "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/probe-request.json"
        ),
        WorkerCapability::Lookup => include_bytes!(
            "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/lookup-request.json"
        ),
        WorkerCapability::Acknowledge => include_bytes!(
            "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/acknowledge-request.json"
        ),
        WorkerCapability::SetControlMode => include_bytes!(
            "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/control-request.json"
        ),
        WorkerCapability::Dispatch => {
            return Ok(AuthenticatedWorkerRequest::from_transport(
                request()?,
                capability,
                WorkerOwnerProof::new("test-owner")?,
            ));
        }
    };
    let mut fields: Map<String, Value> = serde_json::from_slice(fixture)?;
    let dispatch = request()?;
    for (key, value) in &mut fields {
        if !matches!(key.as_str(), "command" | "scope") {
            if let Some(original) = dispatch.fields().get(key) {
                *value = original.clone();
            }
        }
    }
    if let Some(mode) = mode {
        fields.insert("mode".into(), json!(mode));
        fields.insert("mode_sequence".into(), json!(2));
    }
    Ok(AuthenticatedWorkerRequest::from_transport(
        WorkerRequest::decode(&serde_json::to_vec(&fields)?)?,
        capability,
        WorkerOwnerProof::new("test-owner")?,
    ))
}

#[test]
fn quarantine_keeps_history_and_restrictive_control_available()
-> Result<(), Box<dyn std::error::Error>> {
    for (mode, expected) in [
        ("paused", WorkerControlMode::Paused),
        ("draining", WorkerControlMode::Draining),
        ("stopped", WorkerControlMode::Stopped),
    ] {
        let mut runtime = runtime()?;
        let reservation = admitted_reservation(&mut runtime)?;
        runtime.finish_reservation(Some(reservation), ResponseWriteStatus::Failed)?;
        let (probe, _) = runtime
            .handle_authenticated(&command(WorkerCapability::Probe, None)?)?
            .into_parts();
        assert!(matches!(probe, WorkerReply::Probe(probe) if !probe.ready));
        let (lookup, _) = runtime
            .handle_authenticated(&command(WorkerCapability::Lookup, None)?)?
            .into_parts();
        assert!(matches!(lookup, WorkerReply::Lookup(LookupReply::Unknown)));
        let (ack, _) = runtime
            .handle_authenticated(&command(WorkerCapability::Acknowledge, None)?)?
            .into_parts();
        assert!(matches!(
            ack,
            WorkerReply::Acknowledge(AcknowledgmentStatus::Conflict)
        ));
        let (control, _) = runtime
            .handle_authenticated(&command(WorkerCapability::SetControlMode, Some(mode))?)?
            .into_parts();
        assert!(matches!(control, WorkerReply::Control { accepted: true }));
        assert_eq!(
            try_lock_recovery(runtime.store())?
                .worker_control()?
                .ok_or("missing control")?
                .mode,
            expected
        );
        assert!(
            runtime
                .handle_authenticated(&command(WorkerCapability::SetControlMode, Some("running"))?)
                .is_err()
        );
        assert!(
            runtime
                .handle_authenticated(&command(WorkerCapability::Dispatch, None)?)
                .is_err()
        );
        assert!(!runtime.store().admission_open());
    }
    Ok(())
}

#[test]
fn pending_quarantine_probe_is_not_ready_and_capabilities_still_apply()
-> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime()?;
    begin_quarantine(runtime.store())?;
    let (probe, _) = runtime
        .handle_authenticated(&command(WorkerCapability::Probe, None)?)?
        .into_parts();
    assert!(matches!(probe, WorkerReply::Probe(probe) if !probe.ready));
    let control = command(WorkerCapability::SetControlMode, Some("stopped"))?;
    let wrong_capability = AuthenticatedWorkerRequest::from_transport(
        control.request().clone(),
        WorkerCapability::Probe,
        WorkerOwnerProof::new("test-owner")?,
    );
    assert!(runtime.handle_authenticated(&wrong_capability).is_err());
    let mut fields = control.request().fields().clone();
    fields.insert(
        "watchdog_boot_id".into(),
        json!("99999999-9999-4999-8999-999999999999"),
    );
    let stale = AuthenticatedWorkerRequest::from_transport(
        WorkerRequest::decode(&serde_json::to_vec(&fields)?)?,
        WorkerCapability::SetControlMode,
        WorkerOwnerProof::new("test-owner")?,
    );
    assert!(runtime.handle_authenticated(&stale).is_err());
    assert!(runtime.handle_authenticated(&control).is_ok());
    assert!(!runtime.store().admission_open());
    Ok(())
}
