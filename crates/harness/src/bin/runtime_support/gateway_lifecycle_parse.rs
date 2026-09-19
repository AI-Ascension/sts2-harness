// SPDX-License-Identifier: MIT

//! Closed-schema parsers for the gateway lifecycle answers.
//!
//! Every field is read from the exact wire name the gateway's contract
//! defines, and any unknown shape, missing field, or foreign contract or
//! instance is refused rather than coerced. The transport itself stays in
//! `gateway_lifecycle.rs`; this module only turns an answer into a typed value.

use serde_json::Value;
use sts2_harness::management::{
    LaunchProfileId, LifecycleFailure, LifecycleOperationState, LifecycleOperationView,
    LifecycleProcessIdentity, LifecycleState, ManagementError, PROCESS_LIFECYCLE_CONTRACT,
    ProcessLifecycleCapability,
};

pub(super) fn parse_capability(
    value: &Value,
    expected_instance: &str,
) -> Result<ProcessLifecycleCapability, ManagementError> {
    let contract = required_str(value, "contract")?;
    if contract != PROCESS_LIFECYCLE_CONTRACT {
        return Err(ManagementError::unavailable(
            "process_lifecycle_contract_mismatch",
            "gateway advertised an unsupported process-lifecycle contract",
        ));
    }
    let instance_id = required_str(value, "instance_id")?;
    if instance_id != expected_instance {
        return Err(ManagementError::conflict(
            "lifecycle_response_scope_mismatch",
            "gateway advertised a different instance",
        ));
    }
    let available = value
        .get("available")
        .and_then(Value::as_bool)
        .ok_or_else(invalid_response)?;
    let profiles = value
        .get("profiles")
        .and_then(Value::as_array)
        .ok_or_else(invalid_response)?;
    let mut approved = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let raw = profile.as_u64().ok_or_else(invalid_response)?;
        approved.push(LaunchProfileId::new(raw)?);
    }
    Ok(ProcessLifecycleCapability {
        contract: contract.to_owned(),
        available,
        profiles: approved,
        authority_epoch: value.get("authority_epoch").and_then(Value::as_u64),
        instance_id: instance_id.to_owned(),
        unavailable_reason: value
            .get("unavailable_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

pub(super) fn parse_operation(
    value: &Value,
    expected_instance: &str,
) -> Result<LifecycleOperationView, ManagementError> {
    let contract = required_str(value, "contract")?;
    if contract != PROCESS_LIFECYCLE_CONTRACT {
        return Err(ManagementError::unavailable(
            "process_lifecycle_contract_mismatch",
            "gateway answered with an unsupported process-lifecycle contract",
        ));
    }
    let operation_id = value
        .get("operation_id")
        .and_then(Value::as_u64)
        .ok_or_else(invalid_response)?;
    let instance_id = required_str(value, "instance_id")?;
    if instance_id != expected_instance {
        return Err(ManagementError::conflict(
            "lifecycle_response_scope_mismatch",
            "gateway answered for a different instance",
        ));
    }
    let state = parse_state(value.get("state").ok_or_else(invalid_response)?)?;
    let operation_state =
        parse_operation_state(value.get("operation_state").ok_or_else(invalid_response)?)?;
    let authority_epoch = value
        .get("authority_epoch")
        .and_then(Value::as_u64)
        .ok_or_else(invalid_response)?;
    Ok(LifecycleOperationView {
        contract: contract.to_owned(),
        operation_id,
        instance_id: instance_id.to_owned(),
        state,
        operation_state,
        process: parse_process(value.get("process"), instance_id)?,
        authority_epoch,
        failure: parse_failure(value.get("failure"))?,
    })
}

fn parse_process(
    value: Option<&Value>,
    instance_id: &str,
) -> Result<Option<LifecycleProcessIdentity>, ManagementError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let executable = value.get("executable").ok_or_else(invalid_response)?;
    let user_data = value.get("user_data").ok_or_else(invalid_response)?;
    Ok(Some(LifecycleProcessIdentity {
        instance_id: instance_id.to_owned(),
        process: value
            .get("process")
            .and_then(Value::as_u64)
            .ok_or_else(invalid_response)?,
        pid: value
            .get("pid")
            .and_then(Value::as_u64)
            .ok_or_else(invalid_response)?,
        birth_id: value
            .get("birth_id")
            .and_then(Value::as_u64)
            .ok_or_else(invalid_response)?,
        install_id: executable
            .get("install_id")
            .and_then(Value::as_u64)
            .ok_or_else(invalid_response)?,
        executable_id: executable
            .get("executable_id")
            .and_then(Value::as_u64)
            .ok_or_else(invalid_response)?,
        image_id: executable
            .get("image_id")
            .and_then(Value::as_u64)
            .ok_or_else(invalid_response)?,
        namespace_id: user_data
            .get("namespace_id")
            .and_then(Value::as_u64)
            .ok_or_else(invalid_response)?,
    }))
}

fn parse_failure(value: Option<&Value>) -> Result<Option<LifecycleFailure>, ManagementError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    // The gateway retains a stable label. A structured object is accepted only
    // for its `code`; any other shape is refused rather than coerced.
    let code = match value {
        Value::String(code) => code.clone(),
        Value::Object(fields) => fields
            .get("code")
            .and_then(Value::as_str)
            .ok_or_else(invalid_response)?
            .to_owned(),
        _ => return Err(invalid_response()),
    };
    Ok(Some(LifecycleFailure { code, detail: None }))
}

fn parse_state(value: &Value) -> Result<LifecycleState, ManagementError> {
    match value.as_str().ok_or_else(invalid_response)? {
        "Created" => Ok(LifecycleState::Created),
        "Starting" => Ok(LifecycleState::Starting),
        "Ready" => Ok(LifecycleState::Ready),
        "Busy" => Ok(LifecycleState::Busy),
        "Degraded" => Ok(LifecycleState::Degraded),
        "Stopping" => Ok(LifecycleState::Stopping),
        "Stopped" => Ok(LifecycleState::Stopped),
        "Failed" => Ok(LifecycleState::Failed),
        "Unknown" => Ok(LifecycleState::Unknown),
        "Expired" => Ok(LifecycleState::Expired),
        _ => Err(invalid_response()),
    }
}

fn parse_operation_state(value: &Value) -> Result<LifecycleOperationState, ManagementError> {
    match value.as_str().ok_or_else(invalid_response)? {
        "IntentRecorded" => Ok(LifecycleOperationState::IntentRecorded),
        "Starting" => Ok(LifecycleOperationState::Starting),
        "Started" => Ok(LifecycleOperationState::Started),
        "Attached" => Ok(LifecycleOperationState::Attached),
        "Stopping" => Ok(LifecycleOperationState::Stopping),
        "Restarting" => Ok(LifecycleOperationState::Restarting),
        "Stopped" => Ok(LifecycleOperationState::Stopped),
        "Failed" => Ok(LifecycleOperationState::Failed),
        "Blocked" => Ok(LifecycleOperationState::Blocked),
        "Rejected" => Ok(LifecycleOperationState::Rejected),
        "Unknown" => Ok(LifecycleOperationState::Unknown),
        _ => Err(invalid_response()),
    }
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, ManagementError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(invalid_response)
}

fn invalid_response() -> ManagementError {
    ManagementError::unavailable(
        "process_lifecycle_response_invalid",
        "gateway lifecycle answer did not match the closed response schema",
    )
}

/// Maps a transport failure onto the ambiguous lifecycle outcome.
///
/// A lost or unreadable answer cannot distinguish "refused" from "applied", so
/// it is reported as unavailable and the service retains the operation as
/// `Unknown` for reconciliation by identity.
pub(super) fn gateway_error(error: String) -> ManagementError {
    ManagementError::unavailable("process_lifecycle_transport", error)
}
