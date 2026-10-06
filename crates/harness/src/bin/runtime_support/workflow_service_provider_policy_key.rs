// SPDX-License-Identifier: MIT

pub(super) fn valid_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            matches!(byte, b'A'..=b'Z' | b'0'..=b'9' | b'_')
                && (index != 0 || matches!(byte, b'A'..=b'Z' | b'_'))
        })
}

pub(super) fn provider_policy_key(reference: &str) -> Result<[u8; 32], String> {
    let encoded = std::env::var(reference)
        .map_err(|_| format!("provider-policy key reference {reference} is unavailable"))?;
    let bytes = encoded.as_bytes();
    if bytes.len() != 64 {
        return Err(String::from(
            "provider-policy key must be exactly 64 hexadecimal characters",
        ));
    }
    let mut key = [0_u8; 32];
    for (index, slot) in key.iter_mut().enumerate() {
        let high = hex_nibble(bytes[index * 2])
            .ok_or_else(|| String::from("provider-policy key must be hexadecimal"))?;
        let low = hex_nibble(bytes[index * 2 + 1])
            .ok_or_else(|| String::from("provider-policy key must be hexadecimal"))?;
        *slot = (high << 4) | low;
    }
    if key.iter().all(|value| *value == 0) {
        return Err(String::from("provider-policy key must not be all zeroes"));
    }
    Ok(key)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
