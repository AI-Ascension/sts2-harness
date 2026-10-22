// SPDX-License-Identifier: MIT

//! Private inherited-socket frames. These are process-local Harness plumbing, not a public API.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use serde::{Deserialize, Serialize};

use super::{HttpLimits, HttpRequest, HttpResponse, MAX_JSON_BYTES, MAX_RESPONSE_BYTES};

const MAX_FRAME_BYTES: usize = 4 * (MAX_JSON_BYTES + MAX_RESPONSE_BYTES) + 64 * 1024;
const MAX_QUEUED_FRAMES: usize = 96;
const MAX_QUEUED_BYTES: usize = 16 * MAX_FRAME_BYTES;
const IO_TURN_BYTES: usize = 64 * 1024;
pub(super) const MAX_DIAGNOSTIC_BYTES: usize = 4096;
pub(super) const WIRE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ToBackend {
    Hello {
        version: u32,
        generation: u64,
        capacity: usize,
    },
    Dispatch {
        generation: u64,
        sequence: u64,
        deadline_nanos: u64,
        request: Request,
    },
    Diagnostic {
        generation: u64,
        sequence: u64,
        line: String,
    },
    Stop {
        generation: u64,
    },
    #[cfg(test)]
    PanicForTest {
        generation: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum FromBackend {
    Ready {
        version: u32,
        generation: u64,
    },
    Response {
        generation: u64,
        sequence: u64,
        status: u16,
        body: Vec<u8>,
    },
    DiagnosticWritten {
        generation: u64,
        sequence: u64,
    },
    DiagnosticLost {
        generation: u64,
        sequence: u64,
    },
    BackendRetired {
        generation: u64,
        backend_generation: u64,
        reaped: bool,
    },
    SupervisorFailed {
        generation: u64,
        code: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl From<HttpRequest> for Request {
    fn from(request: HttpRequest) -> Self {
        Self {
            method: request.method,
            path: request.path,
            query: request.query,
            headers: request.headers,
            body: request.body,
        }
    }
}

impl Request {
    pub(super) fn into_http(self, limits: &HttpLimits) -> Result<HttpRequest, WireError> {
        let header_bytes = self
            .headers
            .iter()
            .map(|(name, value)| name.len().saturating_add(value.len()))
            .sum::<usize>();
        let query_bytes = self
            .query
            .iter()
            .map(|(name, value)| name.len().saturating_add(value.len()))
            .sum::<usize>();
        if !matches!(self.method.as_str(), "GET" | "POST" | "PUT" | "PATCH")
            || self.path.len().saturating_add(query_bytes) > limits.max_path_bytes
            || header_bytes > limits.max_header_bytes
            || self.body.len() > limits.max_body_bytes
        {
            return Err(WireError::Bounds);
        }
        Ok(HttpRequest {
            method: self.method,
            path: self.path,
            query: self.query,
            headers: self.headers,
            body: self.body,
        })
    }
}

pub(super) fn into_response(
    status: u16,
    body: Vec<u8>,
    limits: &HttpLimits,
) -> Result<HttpResponse, WireError> {
    if !matches!(status, 200 | 400 | 401 | 403 | 404 | 405 | 409 | 422 | 503)
        || body.len() > limits.max_response_bytes
    {
        return Err(WireError::Bounds);
    }
    Ok(HttpResponse {
        status,
        reason: super::response::reason_phrase(status),
        body,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WireError {
    Io,
    Closed,
    Bounds,
    Malformed,
    Capacity,
}

/// A nonblocking, bounded stream; a turn never waits for peer input or backpressure.
pub(super) struct Frames {
    socket: UnixStream,
    input: Vec<u8>,
    output: VecDeque<Vec<u8>>,
    output_offset: usize,
    output_bytes: usize,
}

impl Frames {
    pub(super) fn new(socket: UnixStream) -> Result<Self, WireError> {
        socket.set_nonblocking(true).map_err(|_| WireError::Io)?;
        Ok(Self {
            socket,
            input: Vec::new(),
            output: VecDeque::new(),
            output_offset: 0,
            output_bytes: 0,
        })
    }

    pub(super) fn queue<T: Serialize>(&mut self, message: &T) -> Result<(), WireError> {
        if self.output.len() >= MAX_QUEUED_FRAMES {
            return Err(WireError::Capacity);
        }
        let body = serde_json::to_vec(message).map_err(|_| WireError::Malformed)?;
        if body.is_empty() || body.len() > MAX_FRAME_BYTES {
            return Err(WireError::Bounds);
        }
        if self.output_bytes.saturating_add(4 + body.len()) > MAX_QUEUED_BYTES {
            return Err(WireError::Capacity);
        }
        let length = u32::try_from(body.len()).map_err(|_| WireError::Bounds)?;
        let mut frame = Vec::with_capacity(4 + body.len());
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(&body);
        self.output_bytes += frame.len();
        self.output.push_back(frame);
        Ok(())
    }

    pub(super) fn flush(&mut self) -> Result<(), WireError> {
        let mut remaining = IO_TURN_BYTES;
        while let Some(frame) = self.output.front() {
            let end = frame.len().min(self.output_offset + remaining);
            match self.socket.write(&frame[self.output_offset..end]) {
                Ok(0) => return Err(WireError::Closed),
                Ok(written) => {
                    self.output_offset += written;
                    remaining -= written;
                    if self.output_offset == frame.len() {
                        self.output_bytes -= frame.len();
                        self.output.pop_front();
                        self.output_offset = 0;
                    }
                    if remaining == 0 {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => break,
                Err(_) => return Err(WireError::Io),
            }
        }
        Ok(())
    }

    pub(super) fn output_empty(&self) -> bool {
        self.output.is_empty()
    }

    pub(super) fn receive<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Result<Option<T>, WireError> {
        if let Some(message) = self.decode()? {
            return Ok(Some(message));
        }
        let mut chunk = [0_u8; 8192];
        let mut remaining = IO_TURN_BYTES;
        while remaining != 0 {
            let allowance = chunk.len().min(remaining);
            match self.socket.read(&mut chunk[..allowance]) {
                Ok(0) => return Err(WireError::Closed),
                Ok(read) => {
                    if self.input.len().saturating_add(read) > MAX_FRAME_BYTES + 4 + chunk.len() {
                        return Err(WireError::Bounds);
                    }
                    self.input.extend_from_slice(&chunk[..read]);
                    remaining -= read;
                    if let Some(message) = self.decode()? {
                        return Ok(Some(message));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => break,
                Err(_) => return Err(WireError::Io),
            }
        }
        Ok(None)
    }

    fn decode<T: serde::de::DeserializeOwned>(&mut self) -> Result<Option<T>, WireError> {
        let Some(header) = self.input.get(..4) else {
            return Ok(None);
        };
        let header: [u8; 4] = header.try_into().map_err(|_| WireError::Malformed)?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(WireError::Bounds);
        }
        if self.input.len() < length + 4 {
            return Ok(None);
        }
        let value =
            serde_json::from_slice(&self.input[4..length + 4]).map_err(|_| WireError::Malformed)?;
        self.input.drain(..length + 4);
        Ok(Some(value))
    }
}

#[cfg(test)]
#[path = "http_owned_wire_tests.rs"]
mod tests;
