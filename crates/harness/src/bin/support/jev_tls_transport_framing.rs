// SPDX-License-Identifier: MIT

//! HTTP/1.1 response framing for the System One transport.
//!
//! Split from the transport so the socket, the TLS session, and the credential stay in one file and
//! the rules about *what counts as a whole response* stay in another. That boundary is the one
//! worth having: a transport that returns "whatever followed the header block" cannot tell a whole
//! body from the first half of one, and that is a property of the bytes rather than of the socket.
//!
//! Everything here is a pure function of the bytes that arrived, which is what lets the refusal
//! cases be asserted offline against a literal frame rather than against a server.

use super::{MAX_HEADER_BYTES, MAX_RESPONSE_BYTES};

/// Splits one accepted response and returns its body bytes.
///
/// A status other than `200` is a refusal, and the provider's `429` and `529` are not special:
/// both are reported as a provider status failure, so no caller can mistake a rate limit or an
/// overloaded upstream for a decision.
///
/// The framing is checked rather than assumed, because a transport that hands back "whatever
/// followed the header block" cannot tell a whole body from the first half of one. Exactly one of
/// two shapes is accepted: a declared `Content-Length` that must be present, must be a number, and
/// must be no larger than the body bound, with the body exactly that long; or a `chunked` body
/// that must carry no `Content-Length`, whose decoded size must fit the bound. Anything else --
/// a missing length, a length that disagrees with the bytes, a chunked body that never
/// terminates, a length above the bound, or a body one byte over -- is a refusal.
pub(crate) fn parse_body(raw: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("the provider response had no complete header block")?;
    if split > MAX_HEADER_BYTES {
        return Err("the provider response headers exceeded their bound".into());
    }
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|_| "the provider response headers were not valid text")?;
    if !head.starts_with("HTTP/1.1 200 ") {
        return Err("the provider returned a status other than 200".into());
    }
    let body = &raw[split + 4..];
    if body.len() > MAX_RESPONSE_BYTES {
        return Err("the provider response exceeded its bound".into());
    }
    let headers = HeaderFields::parse(head);
    if headers.chunked {
        if headers.content_length.is_some() {
            return Err("the provider response declared both a length and chunked framing".into());
        }
        decode_chunked(body)
    } else {
        let declared = headers
            .content_length
            .ok_or("the provider response did not declare a length")?;
        if declared > MAX_RESPONSE_BYTES {
            return Err("the provider response declared a length above its bound".into());
        }
        if body.len() != declared {
            return Err("the provider response did not match its declared length".into());
        }
        Ok(body.to_vec())
    }
}

/// The one framing header this transport cares about, resolved to a value.
struct HeaderFields {
    content_length: Option<usize>,
    chunked: bool,
}

impl HeaderFields {
    /// Collects `Content-Length` and the chunked transfer coding from a header block.
    ///
    /// Repeated `Content-Length` fields that disagree make the response ambiguous -- that is a
    /// request-smuggling shape, not something to guess about -- so they are refused by
    /// returning `None` from `content_length` and letting the caller treat the frame as
    /// undeclared-length. Two identical values are harmless and collapse to one.
    fn parse(head: &str) -> Self {
        let mut content_length: Option<usize> = None;
        let mut chunked = false;
        for line in head.split("\r\n").skip(1) {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => {
                    let Ok(value) = value.trim().parse::<usize>() else {
                        // A length that is not a number cannot be checked against the body, so it
                        // is treated as if no usable length were declared.
                        content_length = None;
                        continue;
                    };
                    content_length = Some(match content_length {
                        None => value,
                        Some(previous) if previous == value => value,
                        Some(_) => usize::MAX,
                    });
                }
                "transfer-encoding" if value.to_ascii_lowercase().contains("chunked") => {
                    chunked = true;
                }
                _ => {}
            }
        }
        // `usize::MAX` is the disagreement sentinel; it is above any legal body bound, so it is
        // turned back into "no usable length" and the framing is refused downstream.
        let content_length = content_length.filter(|value| *value != usize::MAX);
        Self {
            content_length,
            chunked,
        }
    }
}

/// Decodes one `chunked` body, refusing a malformed or oversized one.
///
/// Each chunk is a hex size on its own line, then that many bytes, then a `CRLF`; the stream ends
/// at a zero-size chunk. A truncated stream, a size line that is not hex, a size that does not
/// match the bytes that follow, or a decoded total above the body bound are all refusals. The
/// decoded bytes, not the wire bytes, are what the bound is applied to, so a body split into many
/// small chunks is accepted up to the same limit as one large chunk.
fn decode_chunked(body: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut decoded = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("the provider response was truncated inside its chunked framing")?;
        let size_line = std::str::from_utf8(&rest[..line_end])
            .map_err(|_| "the provider response chunk size was not valid text")?;
        // A chunk extension after `;` is legal and ignored, exactly as a conforming reader must.
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| "the provider response chunk size was not a hexadecimal number")?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            // The terminating chunk is followed by trailer headers and a final blank line. Both
            // are optional in the chunked grammar except the final CRLF, so the body is accepted
            // as soon as the zero chunk is seen, which is where the message body ends.
            return Ok(decoded);
        }
        if decoded.len() + size > MAX_RESPONSE_BYTES {
            return Err("the provider response exceeded its bound".into());
        }
        if rest.len() < size + 2 {
            return Err("the provider response was truncated inside a chunk".into());
        }
        decoded.extend_from_slice(&rest[..size]);
        rest = &rest[size..];
        if &rest[..2] != b"\r\n" {
            return Err("the provider response chunk was not terminated".into());
        }
        rest = &rest[2..];
    }
}
