// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};
use sts2_harness::{
    DispatchStatus, EpisodeLegalAction, TransitionReceipt, WaitOutcome, WaitSample,
};

use super::super::config::RuntimeConfig;

pub(crate) fn receipt(
    value: &Value,
    expected_kind: &str,
    config: &RuntimeConfig,
    expected_operation: &str,
    expected_generation: u64,
    action: EpisodeLegalAction,
) -> Result<TransitionReceipt, String> {
    let root = super::root(value, expected_kind, config)?;
    let operation_id = super::string(root, "operation_id")?;
    if operation_id != expected_operation {
        return Err(String::from(
            "Runtime-v3 receipt operation does not match the request",
        ));
    }
    let status = dispatch_status(root)?;
    validate_result_fields(root, status, false)?;
    let after = match status {
        DispatchStatus::Unknown => {
            super::require_null(root, "observation")?;
            super::require_null(root, "legal_actions")?;
            super::require_null(root, "transition")?;
            if root.get("error_code").and_then(Value::as_str).is_none() {
                return Err(String::from(
                    "unknown Runtime-v3 receipt omitted error_code",
                ));
            }
            None
        }
        DispatchStatus::Accepted
        | DispatchStatus::Settled
        | DispatchStatus::Rejected
        | DispatchStatus::Cancelled => Some(super::observation_from_root(root)?),
    };
    let effect_kind = transition(
        root,
        after.as_ref().map(|parsed| &parsed.observation),
        expected_generation,
    )?;
    if status == DispatchStatus::Settled && effect_kind.is_none() {
        return Err(String::from(
            "settled Runtime-v3 receipt omitted transition witness",
        ));
    }
    if status != DispatchStatus::Settled && effect_kind.is_some() {
        return Err(String::from(
            "non-settled Runtime-v3 receipt carried a transition witness",
        ));
    }
    let error_code = root
        .get("error_code")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(TransitionReceipt::new(
        operation_id,
        action,
        status,
        after.map(|parsed| parsed.observation),
        effect_kind,
        error_code,
    ))
}

pub(crate) fn wait_sample(
    value: &Value,
    config: &RuntimeConfig,
    expected_operation: &str,
    expected_generation: u64,
) -> Result<WaitSample, String> {
    let root = super::root(value, "wait_response", config)?;
    let operation_id = super::string(root, "operation_id")?;
    if operation_id != expected_operation {
        return Err(String::from(
            "Runtime-v3 wait operation does not match the request",
        ));
    }
    let outcome = match root.get("wait_outcome").and_then(Value::as_str) {
        Some("successor") => WaitOutcome::Successor,
        Some("same_state_mutation") => WaitOutcome::SameStateMutation,
        Some("timeout") => WaitOutcome::Timeout,
        Some("recovery_required") => WaitOutcome::RecoveryRequired,
        _ => return Err(String::from("Runtime-v3 wait outcome is invalid")),
    };
    let status = dispatch_status(root)?;
    validate_result_fields(root, status, true)?;
    match (status, outcome) {
        (DispatchStatus::Settled, WaitOutcome::Successor | WaitOutcome::SameStateMutation) => {
            let after = super::observation_from_root(root)?;
            let effect_kind = transition(root, Some(&after.observation), expected_generation)?
                .ok_or_else(|| String::from("settled Runtime-v3 wait omitted effect witness"))?;
            Ok(WaitSample::new(outcome, Some(after.observation)).with_effect_kind(effect_kind))
        }
        (DispatchStatus::Unknown, WaitOutcome::Timeout | WaitOutcome::RecoveryRequired) => {
            super::require_null(root, "observation")?;
            super::require_null(root, "legal_actions")?;
            super::require_null(root, "transition")?;
            Ok(WaitSample::new(outcome, None))
        }
        _ => Err(String::from("Runtime-v3 wait status and outcome disagree")),
    }
}

fn transition(
    root: &Map<String, Value>,
    after: Option<&sts2_harness::EpisodeObservation>,
    expected_generation: u64,
) -> Result<Option<String>, String> {
    let Some(value) = root.get("transition") else {
        return Err(String::from("Runtime-v3 response omitted transition field"));
    };
    let Some(object) = value.as_object() else {
        return if value.is_null() {
            Ok(None)
        } else {
            Err(String::from("Runtime-v3 transition is not an object"))
        };
    };
    if object.len() != 4
        || [
            "from_generation",
            "to_generation",
            "state_id",
            "effect_kind",
        ]
        .iter()
        .any(|field| !object.contains_key(*field))
    {
        return Err(String::from("Runtime-v3 transition has an invalid shape"));
    }
    let from = super::number(object, "from_generation")?;
    let to = super::number(object, "to_generation")?;
    let state_id = super::string(object, "state_id")?;
    let effect_kind = super::string(object, "effect_kind")?;
    let after = after.ok_or_else(|| String::from("Runtime-v3 transition lacks observation"))?;
    if from != expected_generation
        || to <= from
        || to != after.generation()
        || state_id != after.state_id()
        || !super::safe_identity(effect_kind)
    {
        return Err(String::from(
            "Runtime-v3 transition witness is inconsistent",
        ));
    }
    Ok(Some(effect_kind.to_owned()))
}

fn dispatch_status(root: &Map<String, Value>) -> Result<DispatchStatus, String> {
    match root.get("status").and_then(Value::as_str) {
        Some("accepted") => Ok(DispatchStatus::Accepted),
        Some("settled") => Ok(DispatchStatus::Settled),
        Some("rejected") => Ok(DispatchStatus::Rejected),
        Some("unknown") => Ok(DispatchStatus::Unknown),
        Some("cancelled") => Ok(DispatchStatus::Cancelled),
        _ => Err(String::from("Runtime-v3 response status is invalid")),
    }
}

fn validate_result_fields(
    root: &Map<String, Value>,
    status: DispatchStatus,
    is_wait: bool,
) -> Result<(), String> {
    for field in ["action", "wait_for_millis", "recovery"] {
        super::require_null(root, field)?;
    }
    if !is_wait {
        super::require_null(root, "wait_outcome")?;
    }
    match status {
        DispatchStatus::Accepted | DispatchStatus::Settled => {
            super::require_null(root, "error_code")
        }
        DispatchStatus::Rejected | DispatchStatus::Cancelled | DispatchStatus::Unknown => {
            super::string(root, "error_code")?;
            Ok(())
        }
    }
}

pub(super) fn validate_installation_fields(
    root: &Map<String, Value>,
    is_wait: bool,
) -> Result<(), String> {
    validate_result_fields(root, dispatch_status(root)?, is_wait)
}
