// SPDX-License-Identifier: MIT

pub(in super::super) fn decode(encoded_value: &str) -> Option<Vec<u8>> {
    const MAX_ENCODED_BYTES: usize = 65_536;
    if encoded_value.is_empty()
        || encoded_value.len() > MAX_ENCODED_BYTES
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
    let data_len = encoded_value.len().checked_sub(padding)?;
    if data_len % 4 == 1 || (padding != 0 && !encoded_value.len().is_multiple_of(4)) {
        return None;
    }
    let decoded_len = data_len.checked_mul(6)?.checked_div(8)?;
    if decoded_len == 0 || decoded_len > sts2_harness::MAX_OPERATION_ACTION_BYTES {
        return None;
    }
    let mut decoded = Vec::with_capacity(decoded_len);
    let (mut accumulator, mut bits) = (0_u32, 0_u32);
    for byte in encoded_value.bytes().take(data_len) {
        accumulator = (accumulator << 6) | u32::from(value(byte)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            decoded.push(u8::try_from(accumulator >> bits).ok()?);
            accumulator &= (1 << bits) - 1;
        }
    }
    (accumulator == 0 && decoded.len() == decoded_len).then_some(decoded)
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

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn accepted_producer_encodings_preserve_bytes_and_reject_bad_padding() {
        assert_eq!(decode("AQI="), Some(vec![1, 2]));
        assert_eq!(decode("AQI"), Some(vec![1, 2]));
        assert_eq!(decode("AQ=="), Some(vec![1]));
        assert_eq!(decode("AQ"), Some(vec![1]));
        for encoded in ["+/8=", "+/8", "-_8=", "-_8"] {
            assert_eq!(decode(encoded), Some(vec![251, 255]));
        }
        for encoded in [
            "AR==", "AR", "AQ-=", "AQ_=", "====", "A", "AQ=", "AQI==", "AQ I", "AQ=I",
        ] {
            assert!(
                decode(encoded).is_none(),
                "accepted invalid encoding {encoded}"
            );
        }
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
