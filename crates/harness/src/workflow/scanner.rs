// SPDX-License-Identifier: MIT

use super::decoder::{DecodeError, DecoderLimits};
use super::diagnostic::{Diagnostic, DiagnosticCode, StructuralLocation};
use std::collections::BTreeSet;

pub(super) fn scan(bytes: &[u8], limits: DecoderLimits) -> Result<(), DecodeError> {
    let mut scanner = Scanner {
        bytes,
        limits,
        position: 0,
    };
    scanner.scan_document()
}

struct Scanner<'a> {
    bytes: &'a [u8],
    limits: DecoderLimits,
    position: usize,
}

impl<'a> Scanner<'a> {
    fn scan_document(&mut self) -> Result<(), DecodeError> {
        self.skip_ws();
        if self.position == self.bytes.len() {
            return Err(self.syntax("$"));
        }
        self.scan_value(0, "$")?;
        self.skip_ws();
        if self.position != self.bytes.len() {
            return Err(DecodeError::Syntax(Diagnostic::error(
                DiagnosticCode::TrailingInput,
                self.location("$"),
            )));
        }
        Ok(())
    }

    fn scan_value(&mut self, depth: usize, path: &str) -> Result<(), DecodeError> {
        if depth > self.limits.max_depth {
            return Err(limit_at(DiagnosticCode::DepthExceeded, path, self.position));
        }
        self.skip_ws();
        let Some(byte) = self.bytes.get(self.position).copied() else {
            return Err(self.syntax(path));
        };
        match byte {
            b'{' => self.scan_object(depth, path),
            b'[' => self.scan_array(depth, path),
            b'"' => self.read_string(path).map(|_| ()),
            b'-' | b'0'..=b'9' => self.scan_number(path),
            b't' => self.scan_literal(b"true", path),
            b'f' => self.scan_literal(b"false", path),
            b'n' => self.scan_literal(b"null", path),
            b'N' | b'I' => Err(DecodeError::Syntax(Diagnostic::error(
                DiagnosticCode::NonFinite,
                self.location(path),
            ))),
            _ => Err(self.syntax(path)),
        }
    }

    fn scan_object(&mut self, depth: usize, path: &str) -> Result<(), DecodeError> {
        self.position += 1;
        self.skip_ws();
        let mut keys = BTreeSet::new();
        let mut count = 0usize;
        if self.bytes.get(self.position) == Some(&b'}') {
            self.position += 1;
            return Ok(());
        }
        loop {
            if count >= self.limits.max_object_members {
                return Err(limit_at(
                    DiagnosticCode::CollectionTooLarge,
                    path,
                    self.position,
                ));
            }
            self.skip_ws();
            let key = self.read_string(path)?;
            if !keys.insert(key) {
                return Err(DecodeError::DuplicateKey(Diagnostic::error(
                    DiagnosticCode::DuplicateKey,
                    self.location(path),
                )));
            }
            self.skip_ws();
            if self.bytes.get(self.position) != Some(&b':') {
                return Err(self.syntax(path));
            }
            self.position += 1;
            let child_path = format!("{path}.member");
            self.scan_value(depth + 1, &child_path)?;
            count += 1;
            self.skip_ws();
            match self.bytes.get(self.position).copied() {
                Some(b',') => {
                    self.position += 1;
                    self.skip_ws();
                    if self.bytes.get(self.position) == Some(&b'}') {
                        return Err(self.syntax(path));
                    }
                }
                Some(b'}') => {
                    self.position += 1;
                    return Ok(());
                }
                _ => return Err(self.syntax(path)),
            }
        }
    }

    fn scan_array(&mut self, depth: usize, path: &str) -> Result<(), DecodeError> {
        self.position += 1;
        self.skip_ws();
        let mut count = 0usize;
        if self.bytes.get(self.position) == Some(&b']') {
            self.position += 1;
            return Ok(());
        }
        loop {
            if count >= self.limits.max_array_items {
                return Err(limit_at(
                    DiagnosticCode::CollectionTooLarge,
                    path,
                    self.position,
                ));
            }
            let child_path = format!("{path}[{count}]");
            self.scan_value(depth + 1, &child_path)?;
            count += 1;
            self.skip_ws();
            match self.bytes.get(self.position).copied() {
                Some(b',') => {
                    self.position += 1;
                    self.skip_ws();
                    if self.bytes.get(self.position) == Some(&b']') {
                        return Err(self.syntax(path));
                    }
                }
                Some(b']') => {
                    self.position += 1;
                    return Ok(());
                }
                _ => return Err(self.syntax(path)),
            }
        }
    }

    fn read_string(&mut self, path: &str) -> Result<String, DecodeError> {
        let start = self.position;
        if self.bytes.get(self.position) != Some(&b'"') {
            return Err(self.syntax(path));
        }
        self.position += 1;
        let mut escaped = false;
        while let Some(byte) = self.bytes.get(self.position).copied() {
            self.position += 1;
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => {
                    let end = self.position;
                    let string = serde_json::from_slice::<String>(&self.bytes[start..end])
                        .map_err(|_| self.syntax(path))?;
                    if string.len() > self.limits.max_string_bytes {
                        return Err(limit_at(DiagnosticCode::StringTooLong, path, self.position));
                    }
                    return Ok(string);
                }
                0..=0x1f => return Err(self.syntax(path)),
                _ => {}
            }
        }
        Err(self.syntax(path))
    }

    fn scan_number(&mut self, path: &str) -> Result<(), DecodeError> {
        let start = self.position;
        if self.bytes.get(self.position) == Some(&b'-') {
            self.position += 1;
        }
        let integer_start = self.position;
        match self.bytes.get(self.position).copied() {
            Some(b'0') => {
                self.position += 1;
                if self
                    .bytes
                    .get(self.position)
                    .is_some_and(u8::is_ascii_digit)
                {
                    return Err(self.syntax(path));
                }
            }
            Some(byte) if byte.is_ascii_digit() => {
                self.position += 1;
                while self
                    .bytes
                    .get(self.position)
                    .is_some_and(u8::is_ascii_digit)
                {
                    self.position += 1;
                }
            }
            _ if self.bytes.get(integer_start) == Some(&b'I') => {
                return Err(DecodeError::Syntax(Diagnostic::error(
                    DiagnosticCode::NonFinite,
                    self.location(path),
                )));
            }
            _ => return Err(self.syntax(path)),
        }
        if self
            .bytes
            .get(self.position)
            .is_some_and(|byte| *byte == b'.' || *byte == b'e' || *byte == b'E')
        {
            return Err(DecodeError::Syntax(Diagnostic::error(
                DiagnosticCode::FloatNotAllowed,
                self.location(path),
            )));
        }
        let token = &self.bytes[start..self.position];
        let parsed = std::str::from_utf8(token)
            .ok()
            .and_then(|text| text.parse::<i128>().ok());
        let Some(value) = parsed else {
            return Err(DecodeError::Syntax(Diagnostic::error(
                DiagnosticCode::UnsafeInteger,
                self.location(path),
            )));
        };
        if value.unsigned_abs() > super::ids::MAX_SAFE_INTEGER as u128 {
            return Err(DecodeError::Syntax(Diagnostic::error(
                DiagnosticCode::UnsafeInteger,
                self.location(path),
            )));
        }
        Ok(())
    }

    fn scan_literal(&mut self, literal: &[u8], path: &str) -> Result<(), DecodeError> {
        let end = self.position.saturating_add(literal.len());
        if self.bytes.get(self.position..end) == Some(literal) {
            self.position = end;
            Ok(())
        } else {
            Err(self.syntax(path))
        }
    }

    fn skip_ws(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.position += 1;
        }
    }

    fn location(&self, path: &str) -> StructuralLocation {
        StructuralLocation {
            path: path.to_owned(),
            offset: u64::try_from(self.position).ok(),
        }
    }

    fn syntax(&self, path: &str) -> DecodeError {
        DecodeError::Syntax(Diagnostic::error(
            DiagnosticCode::InvalidJson,
            self.location(path),
        ))
    }
}

fn limit_at(code: DiagnosticCode, path: &str, offset: usize) -> DecodeError {
    DecodeError::Limit(Diagnostic::error(
        code,
        StructuralLocation {
            path: path.to_owned(),
            offset: u64::try_from(offset).ok(),
        },
    ))
}
