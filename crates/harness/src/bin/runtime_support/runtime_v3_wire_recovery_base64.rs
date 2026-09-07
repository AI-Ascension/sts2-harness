// SPDX-License-Identifier: MIT

const MAX_BASE64_BYTES: usize = 65_536;

pub(super) fn canonical_base64(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_BASE64_BYTES || !value.len().is_multiple_of(4) {
        return false;
    }
    let padding = value.bytes().filter(|byte| *byte == b'=').count();
    if padding > 2 {
        return false;
    }
    let bytes = value.as_bytes();
    let data_end = bytes.len().saturating_sub(padding);
    if bytes[..data_end]
        .iter()
        .any(|byte| base64_value(*byte).is_none())
        || bytes[data_end..].iter().any(|byte| *byte != b'=')
    {
        return false;
    }
    let Some(last) = bytes.chunks_exact(4).last() else {
        return false;
    };
    let Some(second) = base64_value(last[1]) else {
        return false;
    };
    if padding == 2 {
        second & 0x0f == 0
    } else if padding == 1 {
        let Some(third) = base64_value(last[2]) else {
            return false;
        };
        third & 0x03 == 0
    } else {
        true
    }
}

fn base64_value(byte: u8) -> Option<u8> {
    Some(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    })
}
