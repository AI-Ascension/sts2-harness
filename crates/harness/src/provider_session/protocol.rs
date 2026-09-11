// SPDX-License-Identifier: MIT

use super::types::{MAX_FRAME_BYTES, MAX_METHOD_BYTES, NATIVE_FRAME_SCHEMA, SessionError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "protocol_json.rs"]
mod protocol_json;
use protocol_json::parse_strict_json;

/// Typed frame kinds used by the owned fixture/native connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeFrameKind {
    Request,
    Response,
    Notification,
    ServerRequest,
    Error,
}

/// A bounded product envelope around the version-pinned native protocol.  The product never
/// accepts a caller-provided method/params pair; this shape is consumed only by the owned worker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFrame {
    schema: String,
    kind: NativeFrameKind,
    id: Option<u64>,
    method: Option<String>,
    result: Option<Value>,
    error: Option<NativePeerError>,
    sequence: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePeerError {
    pub code: i64,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeResponse {
    Result {
        id: u64,
        value: Value,
    },
    Notification {
        sequence: Option<u64>,
    },
    ServerRequest {
        id: u64,
        method: String,
    },
    Error {
        id: Option<u64>,
        error: NativePeerError,
    },
}

pub fn parse_native_request(line: &[u8]) -> Result<(u64, String, Value), SessionError> {
    if line.is_empty() || line.len() > MAX_FRAME_BYTES || !line.ends_with(b"\n") {
        return Err(SessionError::Protocol);
    }
    let frame: NativeFrame = parse_strict_json(&line[..line.len() - 1])?;
    validate_frame(&frame)?;
    if frame.kind != NativeFrameKind::Request {
        return Err(SessionError::Protocol);
    }
    Ok((
        frame.id.ok_or(SessionError::Protocol)?,
        frame.method.ok_or(SessionError::Protocol)?,
        frame.result.ok_or(SessionError::Protocol)?,
    ))
}

impl NativeFrame {
    #[must_use]
    pub(crate) fn request(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            schema: NATIVE_FRAME_SCHEMA.to_owned(),
            kind: NativeFrameKind::Request,
            id: Some(id),
            method: Some(method.into()),
            result: Some(params),
            error: None,
            sequence: None,
        }
    }

    #[must_use]
    pub fn response(id: u64, result: Value) -> Self {
        Self {
            schema: NATIVE_FRAME_SCHEMA.to_owned(),
            kind: NativeFrameKind::Response,
            id: Some(id),
            method: None,
            result: Some(result),
            error: None,
            sequence: None,
        }
    }

    #[must_use]
    pub fn notification(sequence: u64, result: Value) -> Self {
        Self {
            schema: NATIVE_FRAME_SCHEMA.to_owned(),
            kind: NativeFrameKind::Notification,
            id: None,
            method: None,
            result: Some(result),
            error: None,
            sequence: Some(sequence),
        }
    }

    #[must_use]
    pub fn server_request(id: u64, method: impl Into<String>) -> Self {
        Self {
            schema: NATIVE_FRAME_SCHEMA.to_owned(),
            kind: NativeFrameKind::ServerRequest,
            id: Some(id),
            method: Some(method.into()),
            result: None,
            error: None,
            sequence: None,
        }
    }

    #[must_use]
    pub fn error(id: Option<u64>, code: i64, message: impl Into<String>) -> Self {
        Self {
            schema: NATIVE_FRAME_SCHEMA.to_owned(),
            kind: NativeFrameKind::Error,
            id,
            method: None,
            result: None,
            error: Some(NativePeerError {
                code,
                message: message.into(),
            }),
            sequence: None,
        }
    }

    pub fn encode_line(&self) -> Result<Vec<u8>, SessionError> {
        validate_frame(self)?;
        let mut bytes = serde_json::to_vec(self).map_err(|_| SessionError::Protocol)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(SessionError::Capacity);
        }
        Ok(bytes)
    }
}

pub fn parse_native_frame(line: &[u8]) -> Result<NativeResponse, SessionError> {
    if line.is_empty() || line.len() > MAX_FRAME_BYTES || !line.ends_with(b"\n") {
        return Err(SessionError::Protocol);
    }
    let frame: NativeFrame = parse_strict_json(&line[..line.len() - 1])?;
    validate_frame(&frame)?;
    match frame.kind {
        NativeFrameKind::Response => Ok(NativeResponse::Result {
            id: frame.id.ok_or(SessionError::Protocol)?,
            value: frame.result.unwrap_or(Value::Null),
        }),
        NativeFrameKind::Notification => Ok(NativeResponse::Notification {
            sequence: frame.sequence,
        }),
        NativeFrameKind::ServerRequest => Ok(NativeResponse::ServerRequest {
            id: frame.id.ok_or(SessionError::Protocol)?,
            method: frame.method.ok_or(SessionError::Protocol)?,
        }),
        NativeFrameKind::Error => Ok(NativeResponse::Error {
            id: frame.id,
            error: frame.error.ok_or(SessionError::Protocol)?,
        }),
        NativeFrameKind::Request => Err(SessionError::Protocol),
    }
}

fn validate_frame(frame: &NativeFrame) -> Result<(), SessionError> {
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
