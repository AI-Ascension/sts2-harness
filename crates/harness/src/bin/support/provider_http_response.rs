// SPDX-License-Identifier: MIT

//! Bounded HTTP/1.1 response reading for provider bridges.
//!
//! A provider bridge speaks HTTP to a model service and must decide, from bytes alone, whether a
//! response is well formed before anything downstream reads a decision out of it. That decision is
//! the same for every bridge, so it lives here once rather than in each executable's private support
//! tree: a second copy is a second opinion about what a valid response is.
//!
//! The reader is deliberately strict and fails closed. It accepts one `200` response with either a
//! `Content-Length` body of exactly the declared size or a `chunked` body that terminates properly,
//! and rejects everything else — a non-`200` status, oversized headers, a duplicate length, both
//! framings at once, a transfer coding other than `chunked`, an oversized chunk size, a short or
//! long chunk, and any trailer or trailing byte after the terminal chunk.

use serde_json::Value;

/// Largest response body this reader will accept, in bytes.
pub const MAX_PROVIDER_RESPONSE_BYTES: usize = 128 * 1024;

/// Largest header block this reader will accept, in bytes.
const MAX_HEADER_BYTES: usize = 8192;

/// Largest chunk-size line this reader will accept, in bytes.
const MAX_CHUNK_SIZE_BYTES: usize = 16;

/// Why a provider response was refused.
///
/// Every variant is a refusal: the reader never repairs a malformed response, and the caller cannot
/// distinguish "accepted with a warning" from "accepted".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderResponseError {
    /// The header block had no terminator within its bound.
    MissingHeaders,
    /// The header block exceeded its bound.
    OversizedHeaders,
    /// A header line had no name/value separator, or the status line was unreadable.
    InvalidHeader,
    /// The status line was not `HTTP/1.1 200`.
    Status,
    /// `Content-Length` appeared more than once.
    DuplicateLength,
    /// A transfer coding other than a single `chunked` was declared.
    InvalidTransferEncoding,
    /// `Content-Length` and `Transfer-Encoding` were declared together.
    AmbiguousFraming,
    /// The body length did not match its declared length, or exceeded the bound.
    InvalidLength,
    /// A chunk size line was missing, oversized, or unreadable.
    InvalidChunkSize,
    /// A chunk was short, long, or unterminated, or the body exceeded the bound.
    InvalidChunkLength,
    /// Bytes followed the terminal chunk.
    UnsupportedTrailers,
    /// The body was not valid JSON.
    InvalidJson,
}

impl ProviderResponseError {
    /// Stable machine-readable code for records and diagnostics.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingHeaders => "missing headers",
            Self::OversizedHeaders => "oversized headers",
            Self::InvalidHeader => "invalid header",
            Self::Status => "provider HTTP failure",
            Self::DuplicateLength => "duplicate length",
            Self::InvalidTransferEncoding => "invalid transfer encoding",
            Self::AmbiguousFraming => "ambiguous framing",
            Self::InvalidLength => "invalid response length",
            Self::InvalidChunkSize => "invalid chunk size",
            Self::InvalidChunkLength => "invalid chunk length",
            Self::UnsupportedTrailers => "unsupported trailers or trailing bytes",
            Self::InvalidJson => "invalid response body",
        }
    }
}

impl std::fmt::Display for ProviderResponseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProviderResponseError {}

/// Reads one bounded HTTP/1.1 response and returns its JSON body.
///
/// # Errors
///
/// Returns a [`ProviderResponseError`] for any framing, status, bound, or JSON failure. The reader
/// never returns a partially read body.
pub fn parse_json_response(response: &[u8]) -> Result<Value, ProviderResponseError> {
    let split = response
        .windows(4)
        .position(|pattern| pattern == b"\r\n\r\n")
        .ok_or(ProviderResponseError::MissingHeaders)?;
    if split > MAX_HEADER_BYTES {
        return Err(ProviderResponseError::OversizedHeaders);
    }
    let headers = std::str::from_utf8(&response[..split])
        .map_err(|_| ProviderResponseError::InvalidHeader)?;
    if !headers.starts_with("HTTP/1.1 200 ") {
        return Err(ProviderResponseError::Status);
    }
    let framing = read_framing(headers)?;
    let payload = &response[split + 4..];
    match framing {
        Framing::Chunked => {
            let body = decode_chunks(payload)?;
            serde_json::from_slice(&body).map_err(|_| ProviderResponseError::InvalidJson)
        }
        Framing::Length(length) => {
            if payload.len() > MAX_PROVIDER_RESPONSE_BYTES || length != payload.len() {
                return Err(ProviderResponseError::InvalidLength);
            }
            serde_json::from_slice(payload).map_err(|_| ProviderResponseError::InvalidJson)
        }
    }
}

/// How the body of an accepted response is delimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Framing {
    Length(usize),
    Chunked,
}

/// Reads the one framing the header block declares, refusing an absent or ambiguous declaration.
fn read_framing(headers: &str) -> Result<Framing, ProviderResponseError> {
    let mut length = None;
    let mut chunked = false;
    for line in headers.lines().skip(1) {
        let (name, value) = line
            .split_once(':')
            .ok_or(ProviderResponseError::InvalidHeader)?;
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(ProviderResponseError::DuplicateLength);
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| ProviderResponseError::InvalidLength)?,
            );
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err(ProviderResponseError::InvalidTransferEncoding);
            }
            chunked = true;
        }
    }
    match (chunked, length) {
        (true, Some(_)) => Err(ProviderResponseError::AmbiguousFraming),
        (true, None) => Ok(Framing::Chunked),
        (false, Some(length)) => Ok(Framing::Length(length)),
        (false, None) => Err(ProviderResponseError::InvalidLength),
    }
}

/// Decodes a bounded `chunked` body, refusing trailers and any byte after the terminal chunk.
fn decode_chunks(mut bytes: &[u8]) -> Result<Vec<u8>, ProviderResponseError> {
    let mut result = Vec::new();
    loop {
        let line = bytes
            .windows(2)
            .position(|pattern| pattern == b"\r\n")
            .ok_or(ProviderResponseError::InvalidChunkSize)?;
        if line > MAX_CHUNK_SIZE_BYTES {
            return Err(ProviderResponseError::InvalidChunkSize);
        }
        let size = std::str::from_utf8(&bytes[..line])
            .ok()
            .and_then(|text| usize::from_str_radix(text, 16).ok())
            .ok_or(ProviderResponseError::InvalidChunkSize)?;
        bytes = &bytes[line + 2..];
        if size == 0 {
            if bytes != b"\r\n" {
                return Err(ProviderResponseError::UnsupportedTrailers);
            }
            return Ok(result);
        }
        if size > MAX_PROVIDER_RESPONSE_BYTES.saturating_sub(result.len())
            || bytes.len() < size + 2
            || &bytes[size..size + 2] != b"\r\n"
        {
            return Err(ProviderResponseError::InvalidChunkLength);
        }
        result.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size + 2..];
    }
}

#[cfg(test)]
#[path = "provider_http_response_tests.rs"]
mod tests;
