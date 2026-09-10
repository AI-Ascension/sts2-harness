// SPDX-License-Identifier: MIT

use super::{MAX_JSON_BYTES, parse};

#[test]
fn parser_rejects_duplicate_keys_before_value_construction() -> Result<(), String> {
    for bytes in [
        br#"{"status":"allocated","status":"rejected"}"#.as_slice(),
        br#"{"outer":{"name":1,"name":2}}"#.as_slice(),
        br#"{"a":1,"\u0061":2}"#.as_slice(),
    ] {
        if parse(bytes).is_ok() {
            return Err(String::from("duplicate JSON key was accepted"));
        }
    }
    Ok(())
}

#[test]
fn parser_bounds_bytes_depth_and_trailing_values() -> Result<(), String> {
    let oversized = vec![b' '; MAX_JSON_BYTES + 1];
    if parse(&oversized).is_ok() {
        return Err(String::from("oversized JSON was accepted"));
    }
    let mut nested = Vec::new();
    for _ in 0..65 {
        nested.extend_from_slice(b"[");
    }
    nested.extend(std::iter::repeat_n(b']', 65));
    if parse(&nested).is_ok() {
        return Err(String::from("overly nested JSON was accepted"));
    }
    if parse(br#"{"status":"allocated"}{}"#).is_ok() {
        return Err(String::from("trailing JSON was accepted"));
    }
    Ok(())
}

#[test]
fn parser_preserves_valid_json_values() -> Result<(), String> {
    let value = parse(br#"{"status":"allocated","nested":[true,null,3.5]}"#)?;
    if value["status"] != "allocated"
        || value["nested"][0] != true
        || !value["nested"][2].is_number()
    {
        return Err(String::from("valid JSON value was changed"));
    }
    Ok(())
}
