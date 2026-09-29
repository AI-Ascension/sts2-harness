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
    match Framing::parse(head) {
        // A coding this transport cannot decode means the body cannot be read, so there is no
        // framing to check it against. That is refused before the length question is asked.
        Framing::Unsupported => {
            Err("the provider response used a transfer coding this transport cannot decode".into())
        }
        Framing::Chunked if declared_length(head).is_some() => {
            Err("the provider response declared both a length and chunked framing".into())
        }
        Framing::Chunked => decode_chunked(body),
        Framing::Length(declared) => {
            let declared = declared.ok_or("the provider response did not declare a length")?;
            if declared > MAX_RESPONSE_BYTES {
                return Err("the provider response declared a length above its bound".into());
            }
            if body.len() != declared {
                return Err("the provider response did not match its declared length".into());
            }
            Ok(body.to_vec())
        }
    }
}

/// How a header block declares the shape of the body that follows it.
enum Framing {
    /// A body delimited by a declared `Content-Length`, which may still be absent.
    Length(Option<usize>),
    /// A `chunked` body, decoded and bounded on its decoded bytes.
    Chunked,
    /// A transfer coding this transport cannot decode, so the body cannot be read at all.
    Unsupported,
}

impl Framing {
    /// Resolves the transfer coding and the declared length from a header block.
    ///
    /// `chunked` is the only implemented coding, so any other coding -- including one that merely
    /// has the word inside it -- is a refusal. Comparing whole tokens rather than searching for a
    /// substring is what makes `xchunked` or `chunkedx` a refusal instead of a silently decoded
    /// body, which is the smuggling shape a downstream proxy produces.
    ///
    /// A list is accepted only when it is exactly `chunked`. `gzip, chunked` is legal HTTP -- the
    /// body really is chunked and the `chunked` coding really is last -- but decoding it would mean
    /// silently ignoring the `gzip` step and handing the bridge bytes that are not the body the peer
    /// meant. That is a silent downgrade of the same kind as falling back to plaintext, so it is a
    /// refusal instead. `chunked` after another coding, and a repeated `chunked`, are refused for
    /// the same reason.
    ///
    /// Repeated `Content-Length` fields that disagree make the response ambiguous -- that is a
    /// request-smuggling shape, not something to guess about -- so they are refused by resolving to
    /// `None` and letting the caller treat the frame as undeclared-length. Two identical values
    /// are harmless and collapse to one.
    fn parse(head: &str) -> Self {
        let mut content_length: Option<usize> = None;
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
                "transfer-encoding" => match parse_codings(value) {
                    // An absent or empty `Transfer-Encoding` is not a coding declaration and does
                    // not force the frame into the chunked path.
                    Ok(None) => {}
                    // A declared coding that is not `chunked` last means the body is framed by
                    // something this transport cannot decode. Treating that as "no coding" would
                    // fall through to the length path and refuse for the wrong reason, or -- worse,
                    // decode a body the peer framed some other way.
                    // `parse_codings` only yields `Some` for exactly one `chunked` token, so the
                    // list is known to be non-empty. It is matched rather than unwrapped anyway,
                    // so a future change to that rule refuses the frame instead of panicking.
                    Ok(Some(codings)) => match codings.last() {
                        Some(last) if last == "chunked" => return Self::Chunked,
                        _ => return Self::Unsupported,
                    },
                    Err(()) => return Self::Unsupported,
                },
                _ => {}
            }
        }
        // `usize::MAX` is the disagreement sentinel; it is above any legal body bound, so it is
        // turned back into "no usable length" and the framing is refused downstream.
        let content_length = content_length.filter(|value| *value != usize::MAX);
        Self::Length(content_length)
    }
}

/// Splits one `Transfer-Encoding` value into its lowercased coding tokens.
///
/// Returns `Ok(None)` for a header that declares no coding at all, and `Err(())` when the list
/// cannot be decoded with confidence: a repeated `chunked`, or `chunked` anywhere but last, because
/// in either case the bytes on the wire are not the shape this decoder implements.
fn parse_codings(value: &str) -> Result<Option<Vec<String>>, ()> {
    let codings: Vec<String> = value
        .split(',')
        .map(|coding| coding.trim().to_ascii_lowercase())
        .filter(|coding| !coding.is_empty())
        .collect();
    if codings.is_empty() {
        return Ok(None);
    }
    if codings.len() != 1 || codings[0] != "chunked" {
        // Anything other than exactly one `chunked` coding names a wire format this decoder does
        // not implement.
        return Err(());
    }
    Ok(Some(codings))
}

/// The `Content-Length` a header block declares, if it declares one at all.
///
/// Read separately from [`Framing`] because a frame that is both chunked and length-declared is a
/// refusal rather than a choice of framing: the two disagree, and the disagreement is exactly what
/// this transport has to catch rather than resolve.
fn declared_length(head: &str) -> Option<usize> {
    head.split("\r\n")
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .find_map(|(_, value)| value.trim().parse::<usize>().ok())
}

/// Decodes one `chunked` body, refusing a malformed or oversized one.
///
/// Each chunk is a hex size on its own line, then that many bytes, then a `CRLF`; the stream ends
/// at a zero-size chunk followed by the trailer section and its terminating blank line. A truncated
/// stream, a size line that is not hex, a size that does not match the bytes that follow, a decoded
/// total above the body bound, and a zero chunk with no trailer terminator are all refusals. The
/// decoded bytes, not the wire bytes, are what the bound is applied to, so a body split into many
/// small chunks is accepted up to the same limit as one large chunk.
///
/// The trailer section is required to be well formed even though this transport reads nothing out
/// of it. Accepting the body at the zero chunk and ignoring what follows would accept a frame whose
/// declared grammar is incomplete, which is the same "the first half of a message is a message"
/// mistake the declared-length path refuses.
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
            // The zero chunk is followed by the trailer section: zero or more header lines, then
            // the blank line that ends them. Both are required by the grammar, so a body that
            // stops at `0\r\n` is a truncated message rather than a complete one.
            let trailer_end = rest
                .windows(2)
                .position(|window| window == b"\r\n")
                .ok_or("the provider response ended before its chunked trailers")?;
            let trailers = std::str::from_utf8(&rest[..trailer_end])
                .map_err(|_| "the provider response chunked trailers were not valid text")?;
            if !trailers.is_empty()
                && !trailers.split("\r\n").all(|line| {
                    line.split_once(':')
                        .is_some_and(|(name, _)| !name.trim().is_empty())
                })
            {
                return Err("the provider response chunked trailers were malformed".into());
            }
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
