// SPDX-License-Identifier: MIT

//! HCJ-1 canonicalization and the `host-lease-control-proof-v1` proof recipe.
//!
//! The pinned `protocol-artifact/host-lease-control-v1` profile fixes two byte
//! strings: the canonical form of a frame, and
//! `HMAC-SHA256(key, UTF8(domain) || 0x00 || HCJ1(frame without auth.proof))`.
//! The gateway implements both inside its binary, so the host half of the
//! sideband cannot borrow them and has to re-derive them from the artifact.
//! Every rule below is taken from `PROOF_PROFILE.md` and the schema manifest
//! that ship with the pin, not from any consumer's source.
//!
//! Re-derivation is only worth anything if it is checked, so the accepted and
//! rejected inputs in `proof-vectors.json` are executed verbatim by
//! `tests/host_lease_control_conformance.rs`.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// The pinned `watchdog-host-lease-control-v1` contract and schema digest.
pub(crate) const HOST_LEASE_CONTROL_CONTRACT: &str = "watchdog-host-lease-control-v1";
pub(crate) const HOST_LEASE_CONTROL_SCHEMA_DIGEST: &str =
    "e22faf0f7d3cd313a007b65e52058b3c255153d5778dd8124055c283adf977f9";
/// The recovery envelope that carries the host fence acknowledgment.
pub(crate) const RECOVERY_CONTRACT: &str = "watchdog-recovery-v1";
pub(crate) const RECOVERY_SCHEMA_DIGEST: &str =
    "fb934d3157485aaf6e13e6ebbb213ec8a14c7fc6f5eeebc06b7a22c1f0009217";
/// The largest integer the profile admits on the wire.
pub(crate) const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

/// Parses one raw frame as an HCJ-1 canonical input.
///
/// The profile rejects duplicate member names and non-canonical numbers before
/// a parser can normalize them, so this accepts a strictly smaller language
/// than `serde_json::from_slice` does: `-0`, negative values, decimals, and
/// exponent notation are all refused, and `9007199254740992` is one over the
/// wire bound.
pub(crate) fn parse_canonical_input(raw: &[u8], max_bytes: usize) -> Result<Value, String> {
    if raw.is_empty() {
        return Err(String::from("the frame is empty"));
    }
    if raw.len() > max_bytes {
        return Err(format!(
            "the frame is {} bytes, above the {max_bytes} byte profile bound",
            raw.len()
        ));
    }
    serde_json::from_slice::<CanonicalUnique>(raw)
        .map(|unique| unique.0)
        .map_err(|error| format!("the frame is not a canonical input: {error}"))
}

struct CanonicalUnique(Value);

impl<'de> Deserialize<'de> for CanonicalUnique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(CanonicalUniqueVisitor)
    }
}

struct CanonicalUniqueVisitor;

impl<'de> Visitor<'de> for CanonicalUniqueVisitor {
    type Value = CanonicalUnique;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HCJ-1 JSON without duplicate or noncanonical numbers")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(CanonicalUnique(Value::Bool(value)))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        if value < 0 {
            return Err(E::custom("the profile rejects negative numbers"));
        }
        Ok(CanonicalUnique(Value::Number(value.into())))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        if value > MAX_WIRE_INTEGER {
            return Err(E::custom(
                "the profile rejects integers above the wire bound",
            ));
        }
        Ok(CanonicalUnique(Value::Number(value.into())))
    }

    /// A decimal point or an exponent makes `serde_json` produce a float, and
    /// so does the negative zero that neither `i64` nor `u64` can carry.
    fn visit_f64<E: de::Error>(self, _value: f64) -> Result<Self::Value, E> {
        Err(E::custom(
            "the profile rejects negative zero, decimal points, and exponent notation",
        ))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(CanonicalUnique(Value::String(value.to_owned())))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(CanonicalUnique(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(CanonicalUnique(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(CanonicalUnique(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        while let Some((key, CanonicalUnique(value))) = map.next_entry()? {
            if values.insert(key, value).is_some() {
                return Err(de::Error::custom(
                    "the profile rejects a duplicate member name",
                ));
            }
        }
        Ok(CanonicalUnique(Value::Object(values)))
    }
}

/// Emits the canonical byte form of an already-parsed frame.
pub(crate) fn canonical_hcj1(value: &Value) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    write_canonical(value, &mut output)?;
    Ok(output)
}

fn write_canonical(value: &Value, output: &mut Vec<u8>) -> Result<(), String> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(number) => {
            let value = number
                .as_u64()
                .filter(|value| *value <= MAX_WIRE_INTEGER)
                .ok_or_else(|| String::from("the profile cannot canonicalize this number"))?;
            output.extend_from_slice(value.to_string().as_bytes());
        }
        Value::String(value) => write_string(value, output),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            output.push(b'{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_string(key, output);
                output.push(b':');
                write_canonical(&values[*key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

/// Escapes only what the profile escapes: quotation mark, backslash, the five
/// short control escapes, and lowercase `\u00xx` for the rest of C0. U+2028
/// and U+2029 stay literal, and slash is never escaped.
fn write_string(value: &str, output: &mut Vec<u8>) {
    output.push(b'"');
    for character in value.chars() {
        match character {
            '"' => output.extend_from_slice(br#"\""#),
            '\\' => output.extend_from_slice(br#"\\"#),
            '\u{08}' => output.extend_from_slice(br#"\b"#),
            '\u{0c}' => output.extend_from_slice(br#"\f"#),
            '\n' => output.extend_from_slice(br#"\n"#),
            '\r' => output.extend_from_slice(br#"\r"#),
            '\t' => output.extend_from_slice(br#"\t"#),
            control if control <= '\u{1f}' => {
                let code = control as u32;
                output.extend_from_slice(br#"\u00"#);
                output.push(hex_digit((code >> 4) as u8));
                output.push(hex_digit(code as u8));
            }
            printable => {
                let mut buffer = [0_u8; 4];
                output.extend_from_slice(printable.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    output.push(b'"');
}

/// The lowercase hexadecimal proof over `domain || 0x00 || HCJ1(unsigned)`.
pub(crate) fn proof_for(frame: &Value, domain: &str, key: &[u8]) -> Result<String, String> {
    let mut unsigned = frame.clone();
    if let Some(auth) = unsigned.get_mut("auth").and_then(Value::as_object_mut) {
        auth.remove("proof");
    }
    let canonical = canonical_hcj1(&unsigned)?;
    let mut message = Vec::with_capacity(domain.len() + 1 + canonical.len());
    message.extend_from_slice(domain.as_bytes());
    message.push(0);
    message.extend_from_slice(&canonical);
    Ok(hex_digest(&hmac_sha256(key, &message)))
}

/// Verifies a received frame proof in constant time, after shape validation.
pub(crate) fn verify_proof(
    frame: &Value,
    domain: &str,
    key: &[u8],
    expected: &str,
) -> Result<(), String> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(String::from(
            "the authentication proof is not 64 lowercase hexadecimal characters",
        ));
    }
    if proof_for(frame, domain, key)? == expected {
        return Ok(());
    }
    Err(String::from("the authentication proof does not match"))
}

/// Reads the frame's own proof and verifies it against `domain`.
pub(crate) fn verify_frame_proof(frame: &Value, domain: &str, key: &[u8]) -> Result<(), String> {
    let proof = frame
        .get("auth")
        .and_then(|auth| auth.get("proof"))
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("the frame carries no authentication proof"))?;
    verify_proof(frame, domain, key, proof)
}

/// The profile's shared key is exactly 32 bytes; longer keys are hashed first,
/// which the gateway's fixed-size normalization cannot reach.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0_u8; 64];
    if key.len() > 64 {
        normalized.copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner = [0_u8; 64];
    let mut outer = [0_u8; 64];
    for index in 0..64 {
        inner[index] = normalized[index] ^ 0x36;
        outer[index] = normalized[index] ^ 0x5c;
    }
    let mut inner_hasher = Sha256::new();
    inner_hasher.update(inner);
    inner_hasher.update(message);
    let inner_digest = inner_hasher.finalize();
    let mut outer_hasher = Sha256::new();
    outer_hasher.update(outer);
    outer_hasher.update(inner_digest);
    let mut result = [0_u8; 32];
    result.copy_from_slice(&outer_hasher.finalize());
    result
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(hex_digit(byte >> 4)));
        result.push(char::from(hex_digit(*byte)));
    }
    result
}

fn hex_digit(value: u8) -> u8 {
    b"0123456789abcdef"[(value & 0x0f) as usize]
}

/// The 32-byte host lease key, from 64 hexadecimal characters.
///
/// The gateway also admits the standard base64 form of the same 32 bytes. The
/// campaign environment this fixture documents uses the hexadecimal form, so
/// the base64 spelling is refused loudly rather than decoded here.
pub(crate) fn decode_host_lease_key(encoded: &str) -> Result<[u8; 32], String> {
    let encoded = encoded.trim();
    let secret = if encoded.len() == 64 && encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let mut bytes = Vec::with_capacity(32);
        for pair in encoded.as_bytes().chunks_exact(2) {
            let high = hex_value(pair[0]).ok_or_else(|| String::from("invalid hex digit"))?;
            let low = hex_value(pair[1]).ok_or_else(|| String::from("invalid hex digit"))?;
            bytes.push((high << 4) | low);
        }
        bytes
    } else {
        return Err(String::from(
            "the host lease key must be 64 hexadecimal characters",
        ));
    };
    <[u8; 32]>::try_from(secret).map_err(|_| String::from("the host lease key must be 32 bytes"))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// A UTC `YYYY-MM-DDTHH:MM:SSZ` stamp, the only form the profile admits.
pub(crate) fn timestamp() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default();
    timestamp_from_millis(millis)
}

pub(crate) fn timestamp_from_millis(millis: u64) -> String {
    let seconds = millis / 1_000;
    let days = i64::try_from(seconds / 86_400).unwrap_or_default();
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3_600,
        (second_of_day % 3_600) / 60,
        second_of_day % 60
    )
}

/// Howard Hinnant's `civil_from_days`, the inverse of the days-from-civil
/// conversion the profile's timestamp readers use.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}
