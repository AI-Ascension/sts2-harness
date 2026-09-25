// SPDX-License-Identifier: MIT

//! Byte-level Rust trivia handling shared by the `RUST002` module scanner.
//!
//! These helpers skip whitespace, comments, string literals, raw strings, and
//! character literals so that brackets and keywords inside them cannot change
//! module resolution. They are a bounded heuristic, not a full Rust lexer.

pub(crate) fn trivia(text: &str, mut index: usize) -> usize {
    loop {
        index = skip_space(text, index);
        if text[index..].starts_with("//") {
            index = skip_line(text, index);
        } else if text[index..].starts_with("/*") {
            index = skip_block(text, index);
        } else {
            return index;
        }
    }
}

fn skip_space(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn skip_line(text: &str, index: usize) -> usize {
    match text[index..].find('\n') {
        Some(offset) => index + offset + 1,
        None => text.len(),
    }
}

fn skip_block(text: &str, mut index: usize) -> usize {
    let mut depth = 0;
    while index < text.len() {
        if text[index..].starts_with("/*") {
            depth += 1;
            index += 2;
        } else if text[index..].starts_with("*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index = advance(text, index);
        }
    }
    index
}

/// Returns the index just past the balancing `close` for the opener at `index`.
pub(crate) fn balanced(text: &str, mut index: usize, open: u8, close: u8) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == open {
            depth += 1;
            index += 1;
        } else if byte == close {
            depth -= 1;
            index += 1;
            if depth == 0 {
                return index;
            }
        } else if byte == b'"' {
            index = quoted(text, index);
        } else if byte == b'\'' {
            index = character(text, index);
        } else if text[index..].starts_with("//") {
            index = skip_line(text, index);
        } else if text[index..].starts_with("/*") {
            index = skip_block(text, index);
        } else if byte == b'r' && matches!(bytes.get(index + 1), Some(b'"' | b'#')) {
            index = raw(text, index);
        } else {
            index = advance(text, index);
        }
    }
    index
}

fn quoted(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    index += 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index = advance(text, index),
        }
    }
    index
}

fn character(text: &str, index: usize) -> usize {
    let bytes = text.as_bytes();
    let next = index + 1;
    if bytes.get(next) == Some(&b'\\') {
        // Step past the backslash and the escaped character before looking for
        // the closing quote, so `'\''` consumes its whole span.
        let mut cursor = next + 2;
        while cursor < bytes.len() && bytes[cursor] != b'\'' {
            cursor = advance(text, cursor);
        }
        return (cursor + 1).min(bytes.len());
    }
    if bytes.get(next).copied().is_some_and(ident_start) {
        let (_, after) = ident(text, next);
        return if bytes.get(after) == Some(&b'\'') {
            after + 1
        } else {
            after
        };
    }
    let mut cursor = next;
    while cursor < bytes.len() && bytes[cursor] != b'\'' {
        cursor = advance(text, cursor);
    }
    (cursor + 1).min(bytes.len())
}

fn raw(text: &str, index: usize) -> usize {
    let bytes = text.as_bytes();
    let mut cursor = index + 1;
    let mut hashes = 0;
    while bytes.get(cursor) == Some(&b'#') {
        hashes += 1;
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return index + 1;
    }
    cursor += 1;
    loop {
        if cursor >= bytes.len() {
            return cursor;
        }
        if bytes[cursor] == b'"' {
            let mut after = cursor + 1;
            let mut count = 0;
            while count < hashes && bytes.get(after) == Some(&b'#') {
                count += 1;
                after += 1;
            }
            if count == hashes {
                return after;
            }
        }
        cursor = advance(text, cursor);
    }
}

/// Returns the literal value and end index, or `None` when `index` is not a string.
pub(crate) fn string_literal(text: &str, index: usize) -> Option<(String, usize)> {
    if text.as_bytes().get(index) != Some(&b'"') {
        return None;
    }
    let end = quoted(text, index);
    let closing = end.checked_sub(1)?;
    if closing <= index {
        return None;
    }
    Some((text[index + 1..closing].to_owned(), end))
}

/// Returns the index just past a quoted, raw, or character literal, or `None`
/// when `index` does not begin one. Used to step over literal text so keywords
/// inside it cannot invent a module declaration.
pub(crate) fn literal(text: &str, index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    match bytes.get(index) {
        Some(b'"') => Some(quoted(text, index)),
        Some(b'\'') => Some(character(text, index)),
        Some(b'r') if matches!(bytes.get(index + 1), Some(b'"' | b'#')) => Some(raw(text, index)),
        _ => None,
    }
}

pub(crate) fn ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

pub(crate) fn ident(text: &str, index: usize) -> (String, usize) {
    let bytes = text.as_bytes();
    let mut cursor = index;
    while cursor < bytes.len() && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_') {
        cursor += 1;
    }
    (text[index..cursor].to_owned(), cursor)
}

pub(crate) fn identifier(text: &str, index: usize) -> Option<(String, usize)> {
    if !text.as_bytes().get(index).copied().is_some_and(ident_start) {
        return None;
    }
    Some(ident(text, index))
}

pub(crate) fn advance(text: &str, index: usize) -> usize {
    let bytes = text.as_bytes();
    let mut next = index + 1;
    while next < bytes.len() && bytes[next] & 0b1100_0000 == 0b1000_0000 {
        next += 1;
    }
    next
}
