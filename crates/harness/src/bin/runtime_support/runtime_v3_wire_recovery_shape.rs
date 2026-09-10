// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::values::positive_u53;
use super::{enum_value, exact, object, operation, string};

pub(super) fn validate_payload(value: &Value, kind: &str) -> Result<(), String> {
    let object = object(value, "recovery payload")?;
    match kind {
        "operation_lookup_response" => {
            exact(
                object,
                &["result", "operation", "mutation_authorized"],
                "lookup payload",
            )?;
            validate_result(object.get("result").ok_or("lookup result is missing")?)?;
            if object.get("mutation_authorized") != Some(&Value::Bool(false)) {
                return Err(String::from("lookup mutation_authorized must be false"));
            }
            operation::validate_operation_or_null(object.get("operation"), "lookup operation")
        }
        "operation_reconcile_response" => {
            exact(
                object,
                &["result", "operation", "witness"],
                "reconcile payload",
            )?;
            validate_result(object.get("result").ok_or("reconcile result is missing")?)?;
            operation::validate_operation_or_null(object.get("operation"), "reconcile operation")?;
            if let Some(witness) = object.get("witness")
                && !witness.is_null()
            {
                operation::validate_witness(witness)?;
            }
            Ok(())
        }
        _ => Err(format!("unsupported recovery response kind {kind}")),
    }
}

pub(super) fn validate_result(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery result")?;
    exact(
        object,
        &["status", "retryable", "retry_after_seconds"],
        "recovery result",
    )?;
    enum_value(
        string(object, "status")?,
        &[
            "BOOT_AUTHORITY_CREATED",
            "BOOT_READY",
            "BOOT_BLOCKED",
            "FENCE_ACCEPTED",
            "FENCE_REJECTED",
            "LEASE_ACTIVE",
            "LEASE_RENEWED",
            "LEASE_REVOKED",
            "INTENT_RECORDED",
            "MAY_HAVE_BEEN_DISPATCHED",
            "ACCEPTED",
            "SETTLED",
            "REJECTED",
            "UNKNOWN",
            "RECONCILED",
            "DUPLICATE",
            "CONFLICT",
            "NOT_FOUND",
            "STALE_BOOT",
            "STALE_INCARNATION",
            "STALE_LEASE",
            "LEASE_EXPIRED",
            "AUTH_REQUIRED",
            "FORBIDDEN",
            "CONTRACT_MISMATCH",
            "PERSISTENCE_UNAVAILABLE",
            "HOST_NOT_READY",
            "BOUNDS_EXCEEDED",
            "INVALID",
            "BUSY",
        ],
        "recovery result status",
    )?;
    if !object.get("retryable").is_some_and(Value::is_boolean) {
        return Err(String::from("recovery result retryable is not boolean"));
    }
    match object.get("retry_after_seconds") {
        Some(Value::Null) => Ok(()),
        Some(value) if positive_u53(value).is_some() => Ok(()),
        _ => Err(String::from("recovery retry_after_seconds is invalid")),
    }
}
