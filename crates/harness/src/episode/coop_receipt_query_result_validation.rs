// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::{ReceiptQueryError, ReceiptQueryIdentity, ReceiptQueryReceipt, ReceiptQueryStatus};

const MAX_GENERATION: u64 = 9_007_199_254_740_991;
const RECEIPT_FIELDS: [&str; 7] = [
    "status",
    "after_host_generation",
    "checkpoint_id",
    "state_digest",
    "effect_id",
    "effect_kind",
    "error_code",
];

pub(super) fn parse_receipt(
    value: Option<&Value>,
    status: ReceiptQueryStatus,
    identity: &ReceiptQueryIdentity,
    top_error: Option<&str>,
) -> Result<Option<ReceiptQueryReceipt>, ReceiptQueryError> {
    let Some(value) = value else {
        return Err(ReceiptQueryError::InvalidReceipt);
    };
    if value.is_null() {
        return if matches!(
            status,
            ReceiptQueryStatus::Unknown | ReceiptQueryStatus::RecoveryRequired
        ) {
            Ok(None)
        } else {
            Err(ReceiptQueryError::InvalidReceipt)
        };
    }
    let object = value.as_object().ok_or(ReceiptQueryError::InvalidReceipt)?;
    if object.len() != RECEIPT_FIELDS.len()
        || RECEIPT_FIELDS
            .iter()
            .any(|field| !object.contains_key(*field))
    {
        return Err(ReceiptQueryError::InvalidReceipt);
    }
    let receipt_status = object
        .get("status")
        .and_then(Value::as_str)
        .and_then(ReceiptQueryStatus::parse)
        .ok_or(ReceiptQueryError::InvalidReceipt)?;
    if receipt_status != status
        || matches!(
            status,
            ReceiptQueryStatus::Unknown | ReceiptQueryStatus::RecoveryRequired
        )
    {
        return Err(ReceiptQueryError::ReceiptStatusMismatch);
    }
    let after = optional_generation(object.get("after_host_generation"))?;
    let checkpoint = optional_text(object.get("checkpoint_id"))?;
    let state_digest = optional_digest(object.get("state_digest"))?;
    let effect = optional_text(object.get("effect_id"))?;
    let effect_kind = optional_text(object.get("effect_kind"))?;
    let error_code = optional_text(object.get("error_code"))?;
    let expected_effect = format!("effect:{}", identity.operation_id());
    let expected_kind = format!("{}_settled", identity.action_kind().as_str());
    match status {
        ReceiptQueryStatus::Accepted
            if after.is_some()
                || checkpoint.is_some()
                || state_digest.is_some()
                || effect.is_some()
                || effect_kind.is_some()
                || error_code.is_some()
                || top_error.is_some() =>
        {
            return Err(ReceiptQueryError::InvalidReceipt);
        }
        ReceiptQueryStatus::Settled
            if after.is_none_or(|value| value <= identity.before_host_generation())
                || checkpoint.is_none()
                || state_digest.is_none()
                || effect.as_deref() != Some(expected_effect.as_str())
                || effect_kind.as_deref() != Some(expected_kind.as_str())
                || error_code.is_some()
                || top_error.is_some() =>
        {
            return Err(ReceiptQueryError::InvalidReceipt);
        }
        ReceiptQueryStatus::Rejected
            if after.is_some()
                || checkpoint.is_some()
                || state_digest.is_some()
                || effect.is_some()
                || effect_kind.is_some()
                || error_code.is_none()
                || top_error != error_code.as_deref() =>
        {
            return Err(ReceiptQueryError::InvalidReceipt);
        }
        ReceiptQueryStatus::Unknown | ReceiptQueryStatus::RecoveryRequired => {
            return Err(ReceiptQueryError::ReceiptStatusMismatch);
        }
        _ => {}
    }
    Ok(Some(ReceiptQueryReceipt {
        status,
        after_host_generation: after,
        checkpoint_id: checkpoint,
        state_digest,
        effect_id: effect,
        effect_kind,
        error_code,
    }))
}

pub(super) fn validate_provenance(value: Option<&Value>) -> Result<(), ReceiptQueryError> {
    let object = value
        .and_then(Value::as_object)
        .ok_or(ReceiptQueryError::InvalidProvenance)?;
    if object.len() != 3
        || text(object, "artifact") != Some("sts2-protocol/coop-receipt-query-v1")
        || text(object, "source") != Some("schemas/coop-receipt-query-v1.schema.json")
        || text(object, "generator") != Some("hand-authored")
    {
        return Err(ReceiptQueryError::InvalidProvenance);
    }
    Ok(())
}

pub(super) fn text<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    object.get(field).and_then(Value::as_str)
}

pub(super) fn optional_text(value: Option<&Value>) -> Result<Option<String>, ReceiptQueryError> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if safe(value) => Ok(Some(value.clone())),
        _ => Err(ReceiptQueryError::InvalidIdentity),
    }
}

fn optional_digest(value: Option<&Value>) -> Result<Option<String>, ReceiptQueryError> {
    let value = optional_text(value)?;
    if value.as_deref().is_some_and(|value| {
        value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        Err(ReceiptQueryError::InvalidDigest)
    } else {
        Ok(value)
    }
}

fn optional_generation(value: Option<&Value>) -> Result<Option<u64>, ReceiptQueryError> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(generation(Some(value))?)),
        None => Err(ReceiptQueryError::InvalidGeneration),
    }
}

pub(super) fn generation(value: Option<&Value>) -> Result<u64, ReceiptQueryError> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_GENERATION)
        .ok_or(ReceiptQueryError::InvalidGeneration)
}

fn safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}
