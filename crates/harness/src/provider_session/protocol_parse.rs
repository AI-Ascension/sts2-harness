// SPDX-License-Identifier: MIT

use super::super::types::{MAX_FRAME_BYTES, MAX_METHOD_BYTES, NATIVE_FRAME_SCHEMA, SessionError};
use super::protocol_json::parse_strict_json;
use super::{NativeFrame, NativeFrameKind, NativePeerError, NativeResponse};
use serde_json::{Map, Value};

pub fn parse_native_request(line: &[u8]) -> Result<(u64, String, Value), SessionError> {
    let object = parse_wire_object(line)?;
    if !allowed_keys(&object, &["jsonrpc", "id", "method", "params"])
        || object.get("method").and_then(Value::as_str).is_none()
        || object.get("id").is_none()
        || object.get("result").is_some()
        || object.get("error").is_some()
    {
        return Err(SessionError::Protocol);
    }
    let id = parse_id(object.get("id").ok_or(SessionError::Protocol)?)?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or(SessionError::Protocol)?;
    if !valid_method(method) || method.len() > MAX_METHOD_BYTES {
        return Err(SessionError::Protocol);
    }
    Ok((
        id,
        method.to_owned(),
        object.get("params").cloned().unwrap_or(Value::Null),
    ))
}

pub fn parse_native_frame(line: &[u8]) -> Result<NativeResponse, SessionError> {
    let object = parse_wire_object(line)?;
    let method = object.get("method").and_then(Value::as_str);
    if let Some(method) = method {
        if !allowed_keys(&object, &["jsonrpc", "id", "method", "params"])
            || object.get("result").is_some()
            || object.get("error").is_some()
        {
            return Err(SessionError::Protocol);
        }
        if !valid_method(method) || method.len() > MAX_METHOD_BYTES {
            return Err(SessionError::Protocol);
        }
        if let Some(id) = object.get("id") {
            return Ok(NativeResponse::ServerRequest {
                id: parse_id(id)?,
                method: method.to_owned(),
            });
        }
        let sequence = object
            .get("params")
            .and_then(Value::as_object)
            .and_then(|params| params.get("sequence"))
            .and_then(Value::as_u64);
        return Ok(NativeResponse::Notification { sequence });
    }
    if !allowed_keys(&object, &["jsonrpc", "id", "result", "error"])
        || object.get("method").is_some()
        || (object.get("result").is_some() && object.get("error").is_some())
    {
        return Err(SessionError::Protocol);
    }
    let id = object
        .get("id")
        .map(parse_optional_id)
        .transpose()?
        .flatten();
    if let Some(error) = object.get("error") {
        let error = error.as_object().ok_or(SessionError::Protocol)?;
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .ok_or(SessionError::Protocol)?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .ok_or(SessionError::Protocol)?;
        if message.is_empty() || message.len() > 512 || message.contains('\n') {
            return Err(SessionError::Protocol);
        }
        return Ok(NativeResponse::Error {
            id,
            error: NativePeerError {
                code,
                message: message.to_owned(),
            },
        });
    }
    Ok(NativeResponse::Result {
        id: id.ok_or(SessionError::Protocol)?,
        value: object
            .get("result")
            .cloned()
            .ok_or(SessionError::Protocol)?,
    })
}

fn parse_wire_object(line: &[u8]) -> Result<Map<String, Value>, SessionError> {
    if line.is_empty() || line.len() > MAX_FRAME_BYTES || !line.ends_with(b"\n") {
        return Err(SessionError::Protocol);
    }
    let value: Value = parse_strict_json(&line[..line.len() - 1])?;
    let object = value.as_object().ok_or(SessionError::Protocol)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(SessionError::Protocol);
    }
    Ok(object.clone())
}

fn parse_id(value: &Value) -> Result<u64, SessionError> {
    value
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or(SessionError::Protocol)
}

fn parse_optional_id(value: &Value) -> Result<Option<u64>, SessionError> {
    if value.is_null() {
        Ok(None)
    } else {
        Ok(Some(parse_id(value)?))
    }
}

fn allowed_keys(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

pub(super) fn validate_frame(frame: &NativeFrame) -> Result<(), SessionError> {
    if frame.schema != NATIVE_FRAME_SCHEMA {
        return Err(SessionError::Protocol);
    }
    match frame.kind {
        NativeFrameKind::Request => {
            if frame.id.is_none()
                || frame
                    .method
                    .as_ref()
                    .is_none_or(|m| m.is_empty() || m.len() > MAX_METHOD_BYTES || !valid_method(m))
                || frame.result.is_none()
                || frame.error.is_some()
                || frame.sequence.is_some()
            {
                return Err(SessionError::Protocol);
            }
        }
        NativeFrameKind::Response => {
            if frame.id.is_none()
                || frame.result.is_none()
                || frame.method.is_some()
                || frame.error.is_some()
                || frame.sequence.is_some()
            {
                return Err(SessionError::Protocol);
            }
        }
        NativeFrameKind::Notification => {
            if frame.id.is_some()
                || frame.result.is_none()
                || frame.method.is_some()
                || frame.error.is_some()
                || frame.sequence.is_none()
            {
                return Err(SessionError::Protocol);
            }
        }
        NativeFrameKind::ServerRequest => {
            if frame.id.is_none()
                || frame
                    .method
                    .as_ref()
                    .is_none_or(|m| m.is_empty() || m.len() > MAX_METHOD_BYTES || !valid_method(m))
                || frame.result.is_some()
                || frame.error.is_some()
                || frame.sequence.is_some()
            {
                return Err(SessionError::Protocol);
            }
        }
        NativeFrameKind::Error => {
            if frame.error.is_none()
                || frame.method.is_some()
                || frame.result.is_some()
                || frame.sequence.is_some()
            {
                return Err(SessionError::Protocol);
            }
        }
    }
    if frame.error.as_ref().is_some_and(|error| {
        error.message.is_empty() || error.message.len() > 512 || error.message.contains('\n')
    }) {
        return Err(SessionError::Protocol);
    }
    Ok(())
}

fn valid_method(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
