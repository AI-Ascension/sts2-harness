// SPDX-License-Identifier: MIT

pub(super) fn decode(encoded_value: &str) -> Option<Vec<u8>> {
    const MAX_ENCODED_BYTES: usize = 65_536;
    if encoded_value.is_empty()
        || encoded_value.len() > MAX_ENCODED_BYTES
        || !encoded_value.len().is_multiple_of(4)
        || encoded_value.bytes().any(|byte| byte.is_ascii_whitespace())
    {
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
    let bytes = encoded_value.as_bytes();
    let decoded_len = bytes
        .len()
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)?;
    if decoded_len == 0 || decoded_len > sts2_harness::MAX_OPERATION_ACTION_BYTES {
        return None;
    }
    let mut decoded = Vec::with_capacity(decoded_len);
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
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
        if a >= 64
            || b >= 64
            || (c == 64 && d != 64)
            || (!last && (c == 64 || d == 64))
            || (c == 64 && b & 0x0f != 0)
            || (d == 64 && c & 0x03 != 0)
        {
            return None;
        }
        decoded.push((a << 2) | (b >> 4));
        if c != 64 {
            decoded.push((b << 4) | (c >> 2));
            if d != 64 {
                decoded.push((c << 6) | d);
            }
        }
    }
    (decoded.len() == decoded_len).then_some(decoded)
}

fn value(byte: u8) -> Option<u8> {
    Some(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn canonical_base64_rejects_url_alphabet_and_noncanonical_padding() {
        assert_eq!(decode("AQI="), Some(vec![1, 2]));
        assert!(decode("AQI").is_none());
        assert!(decode("AR==").is_none());
        assert!(decode("AQ-=").is_none());
        assert!(decode("AQ_=").is_none());
        assert!(decode("====").is_none());
    }

    #[test]
    fn encoded_and_decoded_bounds_are_checked_before_copy() {
        assert!(decode(&"A".repeat(65_537)).is_none());
        assert_eq!(
            decode(&"A".repeat(65_536)).map(|bytes| bytes.len()),
            Some(49_152)
        );
    }
}
