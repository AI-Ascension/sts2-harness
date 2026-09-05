// SPDX-License-Identifier: MIT

use serde_json::Value;

const LIMIT: usize = 128 * 1024;

pub(super) fn parse(response: &[u8]) -> Result<Value, Box<dyn std::error::Error>> {
    let split = response
        .windows(4)
        .position(|p| p == b"\r\n\r\n")
        .ok_or("missing headers")?;
    if split > 8192 {
        return Err("oversized headers".into());
    }
    let headers = std::str::from_utf8(&response[..split])?;
    if !headers.starts_with("HTTP/1.1 200 ") {
        return Err("provider HTTP failure".into());
    }
    let mut length = None;
    let mut chunked = false;
    for line in headers.lines().skip(1) {
        let (name, value) = line.split_once(':').ok_or("invalid header")?;
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err("duplicate length".into());
            }
            length = Some(value.trim().parse::<usize>()?);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err("invalid transfer encoding".into());
            }
            chunked = true;
        }
    }
    let payload = &response[split + 4..];
    if chunked {
        if length.is_some() {
            return Err("ambiguous framing".into());
        }
        return Ok(serde_json::from_slice(&decode_chunks(payload)?)?);
    }
    if payload.len() > LIMIT || length != Some(payload.len()) {
        return Err("invalid response length".into());
    }
    Ok(serde_json::from_slice(payload)?)
}

fn decode_chunks(mut bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut result = Vec::new();
    loop {
        let line = bytes
            .windows(2)
            .position(|v| v == b"\r\n")
            .ok_or("missing chunk size")?;
        if line > 16 {
            return Err("oversized chunk size".into());
        }
        let size = usize::from_str_radix(std::str::from_utf8(&bytes[..line])?, 16)?;
        bytes = &bytes[line + 2..];
        if size == 0 {
            if bytes != b"\r\n" {
                return Err("unsupported trailers or trailing bytes".into());
            }
            return Ok(result);
        }
        if size > LIMIT - result.len()
            || bytes.len() < size + 2
            || &bytes[size..size + 2] != b"\r\n"
        {
            return Err("invalid chunk length".into());
        }
        result.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_chunked_framing_without_accepting_ambiguity() {
        assert_eq!(
            decode_chunks(b"2\r\n{}\r\n0\r\n\r\n").ok(),
            Some(b"{}".to_vec())
        );
        for bytes in [
            b"3\r\n{}\r\n0\r\n\r\n".as_slice(),
            b"0\r\n\r\nextra",
            b"fffffff\r\n",
        ] {
            assert!(decode_chunks(bytes).is_err());
        }
        assert!(
            parse(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n{}")
                .is_err()
        );
    }
}
