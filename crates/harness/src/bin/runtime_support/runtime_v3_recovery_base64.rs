// SPDX-License-Identifier: MIT

pub(super) fn decode(encoded_value: &str) -> Option<Vec<u8>> {
    if encoded_value.is_empty() || encoded_value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return None;
    }
    let padding = encoded_value.bytes().filter(|byte| *byte == b'=').count();
    if padding > 2
        || encoded_value
            .bytes()
            .take(encoded_value.len().saturating_sub(padding))
            .any(|byte| byte == b'=')
    {
        return None;
    }
    let mut encoded = encoded_value.as_bytes().to_vec();
    if padding == 0 {
        match encoded.len() % 4 {
            0 => {}
            2 => encoded.extend_from_slice(b"=="),
            3 => encoded.push(b'='),
            _ => return None,
        }
    } else if !encoded.len().is_multiple_of(4) {
        return None;
    }
    let mut decoded = Vec::with_capacity(encoded.len() / 4 * 3);
    for (index, chunk) in encoded.chunks_exact(4).enumerate() {
        let last = index + 1 == encoded.len() / 4;
        let a = value(chunk[0])?;
        let b = value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            64
        } else {
            value(chunk[2])?
        };
        let d = if chunk[3] == b'=' {
            64
        } else {
            value(chunk[3])?
        };
        if a >= 64 || b >= 64 || (c == 64 && d != 64) || (!last && (c == 64 || d == 64)) {
            return None;
        }
        decoded.push((a << 2) | (b >> 4));
        if c != 64 {
            decoded.push((b << 4) | (c >> 2));
            if d != 64 {
                decoded.push((c << 6) | d);
            }
        }
        if decoded.len() > sts2_harness::MAX_OPERATION_ACTION_BYTES {
            return None;
        }
    }
    Some(decoded)
}

fn value(byte: u8) -> Option<u8> {
    Some(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    })
}
