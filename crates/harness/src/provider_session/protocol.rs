// SPDX-License-Identifier: MIT

use super::types::{MAX_FRAME_BYTES, NATIVE_FRAME_SCHEMA, SessionError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[path = "protocol_json.rs"]
mod protocol_json;
#[path = "protocol_parse.rs"]
mod protocol_parse;
use protocol_parse::validate_frame;
pub use protocol_parse::{parse_native_frame, parse_native_request};

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
#[derive(Clone, Debug, Eq, PartialEq)]
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
        let mut object = Map::new();
        object.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
        match self.kind {
            NativeFrameKind::Request => {
                object.insert(
                    "id".to_owned(),
                    json!(self.id.ok_or(SessionError::Protocol)?),
                );
                object.insert(
                    "method".to_owned(),
                    Value::String(self.method.clone().ok_or(SessionError::Protocol)?),
                );
                object.insert(
                    "params".to_owned(),
                    self.result.clone().unwrap_or(Value::Null),
                );
            }
            NativeFrameKind::Response => {
                object.insert(
                    "id".to_owned(),
                    json!(self.id.ok_or(SessionError::Protocol)?),
                );
                object.insert(
                    "result".to_owned(),
                    self.result.clone().unwrap_or(Value::Null),
                );
            }
            NativeFrameKind::Notification => {
                object.insert(
                    "method".to_owned(),
                    Value::String("codex/notification".to_owned()),
                );
                object.insert(
                    "params".to_owned(),
                    json!({
                        "sequence": self.sequence.ok_or(SessionError::Protocol)?,
                        "payload": self.result.clone().unwrap_or(Value::Null),
                    }),
                );
            }
            NativeFrameKind::ServerRequest => {
                object.insert(
                    "id".to_owned(),
                    json!(self.id.ok_or(SessionError::Protocol)?),
                );
                object.insert(
                    "method".to_owned(),
                    Value::String(self.method.clone().ok_or(SessionError::Protocol)?),
                );
                object.insert("params".to_owned(), Value::Object(Map::new()));
            }
            NativeFrameKind::Error => {
                object.insert("id".to_owned(), self.id.map_or(Value::Null, |id| json!(id)));
                let peer_error = self.error.clone().ok_or(SessionError::Protocol)?;
                object.insert(
                    "error".to_owned(),
                    json!({"code": peer_error.code, "message": peer_error.message}),
                );
            }
        }
        let mut bytes =
            serde_json::to_vec(&Value::Object(object)).map_err(|_| SessionError::Protocol)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(SessionError::Capacity);
        }
        Ok(bytes)
    }
}
