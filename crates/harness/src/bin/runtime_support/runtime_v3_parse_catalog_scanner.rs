// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::Value;

const MAX_JSON_DEPTH: usize = 64;

pub(super) fn root_catalog(bytes: &[u8]) -> Result<Option<(usize, usize)>, String> {
    Scanner::new(bytes).root_catalog()
}

struct Scanner<'a> {
    bytes: &'a [u8],
    cursor: usize,
    depth: usize,
}

impl<'a> Scanner<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            cursor: 0,
            depth: 0,
        }
    }

    fn root_catalog(mut self) -> Result<Option<(usize, usize)>, String> {
        self.skip_whitespace();
        if self.take() != Some(b'{') {
            return Err(String::from("Runtime-v3 MCP content is not a JSON object"));
        }
        let mut fields = BTreeSet::new();
        let mut catalog = None;
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.cursor += 1;
        } else {
            loop {
                let key_start = self.cursor_after_whitespace();
                let key_end = self.parse_string()?;
                let key: String = serde_json::from_slice(&self.bytes[key_start..key_end])
                    .map_err(|_| String::from("Runtime-v3 MCP member name is invalid"))?;
                if !fields.insert(key.clone()) {
                    return Err(String::from("Runtime-v3 MCP content has duplicate members"));
                }
                self.skip_whitespace();
                if self.take() != Some(b':') {
                    return Err(String::from("Runtime-v3 MCP object omitted a colon"));
                }
                let value_start = self.cursor_after_whitespace();
                let value_end = self.parse_value()?;
                if key == "legal_actions" {
                    if self.bytes.get(value_start) != Some(&b'[') || catalog.is_some() {
                        return Err(String::from(
                            "Runtime-v3 legal_actions member is ambiguous or not an array",
                        ));
                    }
                    catalog = Some((value_start, value_end));
                }
                self.skip_whitespace();
                match self.take() {
                    Some(b',') => {}
                    Some(b'}') => break,
                    _ => {
                        return Err(String::from(
                            "Runtime-v3 MCP object has an invalid separator",
                        ));
                    }
                }
            }
        }
        self.skip_whitespace();
        if self.cursor != self.bytes.len() {
            return Err(String::from("Runtime-v3 MCP content has trailing bytes"));
        }
        Ok(catalog)
    }

    fn parse_value(&mut self) -> Result<usize, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string(),
            Some(b't') => self.parse_literal(b"true"),
            Some(b'f') => self.parse_literal(b"false"),
            Some(b'n') => self.parse_literal(b"null"),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            _ => Err(String::from(
                "Runtime-v3 MCP content has an invalid JSON value",
            )),
        }
    }

    fn parse_object(&mut self) -> Result<usize, String> {
        self.enter_depth()?;
        self.cursor += 1;
        let mut fields = BTreeSet::new();
        self.skip_whitespace();
        if self.take() == Some(b'}') {
            self.leave_depth();
            return Ok(self.cursor);
        }
        self.cursor = self.cursor.saturating_sub(1);
        loop {
            let key_start = self.cursor_after_whitespace();
            let key_end = self.parse_string()?;
            let key: String = serde_json::from_slice(&self.bytes[key_start..key_end])
                .map_err(|_| String::from("Runtime-v3 nested member name is invalid"))?;
            if !fields.insert(key) {
                return Err(String::from(
                    "Runtime-v3 nested object has duplicate members",
                ));
            }
            self.skip_whitespace();
            if self.take() != Some(b':') {
                return Err(String::from("Runtime-v3 nested object omitted a colon"));
            }
            self.parse_value()?;
            self.skip_whitespace();
            match self.take() {
                Some(b',') => {}
                Some(b'}') => break,
                _ => {
                    return Err(String::from(
                        "Runtime-v3 nested object has an invalid separator",
                    ));
                }
            }
        }
        self.leave_depth();
        Ok(self.cursor)
    }

    fn parse_array(&mut self) -> Result<usize, String> {
        self.enter_depth()?;
        self.cursor += 1;
        self.skip_whitespace();
        if self.take() == Some(b']') {
            self.leave_depth();
            return Ok(self.cursor);
        }
        self.cursor = self.cursor.saturating_sub(1);
        loop {
            self.parse_value()?;
            self.skip_whitespace();
            match self.take() {
                Some(b',') => {}
                Some(b']') => break,
                _ => {
                    return Err(String::from(
                        "Runtime-v3 JSON array has an invalid separator",
                    ));
                }
            }
        }
        self.leave_depth();
        Ok(self.cursor)
    }

    fn parse_string(&mut self) -> Result<usize, String> {
        if self.take() != Some(b'"') {
            return Err(String::from(
                "Runtime-v3 JSON string is missing its opening quote",
            ));
        }
        while let Some(byte) = self.take() {
            match byte {
                b'"' => return Ok(self.cursor),
                b'\\' => {
                    let escaped = self.take().ok_or_else(|| {
                        String::from("Runtime-v3 JSON string has a truncated escape")
                    })?;
                    if escaped == b'u' {
                        for _ in 0..4 {
                            if !self.take().is_some_and(|value| value.is_ascii_hexdigit()) {
                                return Err(String::from(
                                    "Runtime-v3 JSON string has an invalid unicode escape",
                                ));
                            }
                        }
                    }
                }
                value if value < 0x20 => {
                    return Err(String::from(
                        "Runtime-v3 JSON string contains a control byte",
                    ));
                }
                _ => {}
            }
        }
        Err(String::from("Runtime-v3 JSON string is unterminated"))
    }

    fn parse_literal(&mut self, literal: &[u8]) -> Result<usize, String> {
        let end = self
            .cursor
            .checked_add(literal.len())
            .ok_or_else(|| String::from("Runtime-v3 JSON value length overflowed"))?;
        if self.bytes.get(self.cursor..end) != Some(literal) {
            return Err(String::from("Runtime-v3 JSON literal is invalid"));
        }
        self.cursor = end;
        Ok(end)
    }

    fn parse_number(&mut self) -> Result<usize, String> {
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}'))
        {
            self.cursor += 1;
        }
        let end = self.cursor;
        if serde_json::from_slice::<Value>(&self.bytes[start..end]).is_err() {
            return Err(String::from("Runtime-v3 JSON number is invalid"));
        }
        Ok(end)
    }

    fn enter_depth(&mut self) -> Result<(), String> {
        self.depth = self
            .depth
            .checked_add(1)
            .ok_or_else(|| String::from("Runtime-v3 JSON depth overflowed"))?;
        if self.depth > MAX_JSON_DEPTH {
            return Err(String::from("Runtime-v3 MCP content is too deeply nested"));
        }
        Ok(())
    }

    fn leave_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.cursor += 1;
        }
    }

    fn cursor_after_whitespace(&mut self) -> usize {
        self.skip_whitespace();
        self.cursor
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.cursor += 1;
        Some(byte)
    }
}
