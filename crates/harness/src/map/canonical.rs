// SPDX-License-Identifier: MIT

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const MAX_CANONICAL_JSON_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CANONICAL_JSON_DEPTH: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalError {
    TooLarge,
    DuplicateKey,
    InvalidJson,
    Serialization,
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "canonical JSON exceeds its bound",
            Self::DuplicateKey => "canonical JSON contains a duplicate object key",
            Self::InvalidJson => "canonical JSON is malformed",
            Self::Serialization => "canonical JSON serialization failed",
        })
    }
}

impl std::error::Error for CanonicalError {}

pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let json = serde_json::to_value(value).map_err(|_| CanonicalError::Serialization)?;
    let mut output = String::new();
    write_value(&json, &mut output)?;
    if output.len() > MAX_CANONICAL_JSON_BYTES {
        return Err(CanonicalError::TooLarge);
    }
    Ok(output.into_bytes())
}

pub fn canonical_digest<T: Serialize>(value: &T) -> Result<String, CanonicalError> {
    Ok(crate::hex_bytes(Sha256::digest(canonical_bytes(value)?)))
}

pub fn reject_duplicate_keys(bytes: &[u8]) -> Result<(), CanonicalError> {
    if bytes.len() > MAX_CANONICAL_JSON_BYTES {
        return Err(CanonicalError::TooLarge);
    }
    let mut scanner = Scanner {
        bytes,
        position: 0,
        depth: 0,
    };
    scanner.value()?;
    scanner.whitespace();
    if scanner.position == bytes.len() {
        Ok(())
    } else {
        Err(CanonicalError::InvalidJson)
    }
}

fn write_value(value: &Value, output: &mut String) -> Result<(), CanonicalError> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output
            .push_str(&serde_json::to_string(value).map_err(|_| CanonicalError::Serialization)?),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_value(value, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            output.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).map_err(|_| CanonicalError::Serialization)?,
                );
                output.push(':');
                let value = values.get(key).ok_or(CanonicalError::Serialization)?;
                write_value(value, output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

struct Scanner<'a> {
    bytes: &'a [u8],
    position: usize,
    depth: usize,
}

impl Scanner<'_> {
    fn value(&mut self) -> Result<(), CanonicalError> {
        self.whitespace();
        match self.bytes.get(self.position).copied() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(|_| ()),
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'n') => self.literal(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(CanonicalError::InvalidJson),
        }
    }

    fn object(&mut self) -> Result<(), CanonicalError> {
        self.enter()?;
        self.position += 1;
        self.whitespace();
        let mut keys = BTreeSet::new();
        if self.consume(b'}') {
            self.leave();
            return Ok(());
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            let key: String =
                serde_json::from_slice(key).map_err(|_| CanonicalError::InvalidJson)?;
            if !keys.insert(key) {
                return Err(CanonicalError::DuplicateKey);
            }
            self.whitespace();
            if !self.consume(b':') {
                return Err(CanonicalError::InvalidJson);
            }
            self.value()?;
            self.whitespace();
            if self.consume(b'}') {
                self.leave();
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(CanonicalError::InvalidJson);
            }
        }
    }

    fn array(&mut self) -> Result<(), CanonicalError> {
        self.enter()?;
        self.position += 1;
        self.whitespace();
        if self.consume(b']') {
            self.leave();
            return Ok(());
        }
        loop {
            self.value()?;
            self.whitespace();
            if self.consume(b']') {
                self.leave();
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(CanonicalError::InvalidJson);
            }
        }
    }

    fn string(&mut self) -> Result<&[u8], CanonicalError> {
        let start = self.position;
        if !self.consume(b'"') {
            return Err(CanonicalError::InvalidJson);
        }
        while let Some(byte) = self.bytes.get(self.position).copied() {
            match byte {
                b'"' => {
                    self.position += 1;
                    return Ok(&self.bytes[start..self.position]);
                }
                b'\\' => {
                    self.position = self.position.saturating_add(2);
                }
                0..=0x1f => return Err(CanonicalError::InvalidJson),
                _ => self.position += 1,
            }
        }
        Err(CanonicalError::InvalidJson)
    }

    fn literal(&mut self, literal: &[u8]) -> Result<(), CanonicalError> {
        let end = self.position.saturating_add(literal.len());
        if self.bytes.get(self.position..end) == Some(literal) {
            self.position = end;
            Ok(())
        } else {
            Err(CanonicalError::InvalidJson)
        }
    }

    fn number(&mut self) -> Result<(), CanonicalError> {
        let start = self.position;
        while let Some(byte) = self.bytes.get(self.position).copied() {
            if byte.is_ascii_whitespace() || matches!(byte, b',' | b']' | b'}') {
                break;
            }
            self.position += 1;
        }
        serde_json::from_slice::<Value>(&self.bytes[start..self.position])
            .map(|_| ())
            .map_err(|_| CanonicalError::InvalidJson)
    }

    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.position += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn enter(&mut self) -> Result<(), CanonicalError> {
        self.depth += 1;
        if self.depth > MAX_CANONICAL_JSON_DEPTH {
            Err(CanonicalError::InvalidJson)
        } else {
            Ok(())
        }
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }
}
