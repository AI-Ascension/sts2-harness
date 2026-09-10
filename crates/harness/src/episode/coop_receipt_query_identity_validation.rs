// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::{ReceiptQueryCoordinate, ReceiptQueryIdentityError, ReceiptQueryLocation};

pub(super) const MAX_GENERATION: u64 = 9_007_199_254_740_991;
const MAX_ID: usize = 128;

pub(super) fn parse_location(
    value: Option<&Value>,
) -> Result<ReceiptQueryLocation, ReceiptQueryIdentityError> {
    let object = value
        .and_then(Value::as_object)
        .ok_or(ReceiptQueryIdentityError::InvalidLocation)?;
    if object.len() != 3
        || ["act_index", "room_id", "coord"]
            .iter()
            .any(|field| !object.contains_key(*field))
    {
        return Err(ReceiptQueryIdentityError::InvalidLocation);
    }
    let coordinate = match object.get("coord") {
        Some(Value::Null) => None,
        Some(Value::Object(coord))
            if coord.len() == 2 && coord.contains_key("col") && coord.contains_key("row") =>
        {
            Some(ReceiptQueryCoordinate::new(
                signed(coord.get("col"))?,
                signed(coord.get("row"))?,
            ))
        }
        _ => return Err(ReceiptQueryIdentityError::InvalidLocation),
    };
    Ok(ReceiptQueryLocation::new(
        signed(object.get("act_index"))?,
        optional_signed(object.get("room_id"))?,
        coordinate,
    ))
}

pub(super) fn text<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, ReceiptQueryIdentityError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| safe(value))
        .ok_or(ReceiptQueryIdentityError::InvalidIdentity)
}

pub(super) fn digest(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, ReceiptQueryIdentityError> {
    let value = text(object, field)?;
    hex(value)
        .then(|| value.to_owned())
        .ok_or(ReceiptQueryIdentityError::InvalidDigest)
}

pub(super) fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn generation(value: Option<&Value>) -> Result<u64, ReceiptQueryIdentityError> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_GENERATION)
        .ok_or(ReceiptQueryIdentityError::InvalidGeneration)
}

fn signed(value: Option<&Value>) -> Result<i32, ReceiptQueryIdentityError> {
    value
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(ReceiptQueryIdentityError::InvalidLocation)
}

fn optional_signed(value: Option<&Value>) -> Result<Option<i32>, ReceiptQueryIdentityError> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(signed(Some(value))?)),
        None => Err(ReceiptQueryIdentityError::InvalidLocation),
    }
}

pub(super) fn safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}
