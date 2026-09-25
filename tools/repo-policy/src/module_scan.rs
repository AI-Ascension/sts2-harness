// SPDX-License-Identifier: MIT

//! Bounded Rust source scan for the `RUST002` module-reachability rule.
//!
//! Only file-resolution syntax is recognised: `mod name;`, `mod name { }`,
//! `#[path = "..."]`, `#[cfg_attr(..., path = "...")]`, and `include!("...")`.
//! Comments and string literals are skipped so text inside them cannot invent a
//! module edge.

use crate::module_lexer::{
    advance, balanced, ident, ident_start, identifier, literal, string_literal, trivia,
};

pub(crate) struct Declaration {
    pub(crate) name: String,
    pub(crate) paths: Vec<String>,
    pub(crate) semi: bool,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) fn scan(text: &str) -> (Vec<Declaration>, Vec<String>) {
    let bytes = text.as_bytes();
    let mut declarations = Vec::new();
    let mut includes = Vec::new();
    let mut attributes: Vec<String> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        index = trivia(text, index);
        if index >= bytes.len() {
            break;
        }
        if let Some(end) = literal(text, index) {
            attributes.clear();
            index = end;
            continue;
        }
        if bytes[index] == b'#' {
            if bytes.get(index + 1) == Some(&b'[') {
                let end = balanced(text, index + 1, b'[', b']');
                attributes.push(text[index..end].to_owned());
                index = end;
            } else if bytes.get(index + 1) == Some(&b'!') && bytes.get(index + 2) == Some(&b'[') {
                index = balanced(text, index + 2, b'[', b']');
            } else {
                attributes.clear();
                index += 1;
            }
            continue;
        }
        if !ident_start(bytes[index]) {
            attributes.clear();
            index = advance(text, index);
            continue;
        }
        let (word, next) = ident(text, index);
        match word.as_str() {
            "pub" => {
                let after = trivia(text, next);
                index = if bytes.get(after) == Some(&b'(') {
                    balanced(text, after, b'(', b')')
                } else {
                    next
                };
            }
            "mod" => {
                let start = index;
                let after = trivia(text, next);
                let Some((name, name_end)) = identifier(text, after) else {
                    attributes.clear();
                    index = next;
                    continue;
                };
                let terminator = trivia(text, name_end);
                match bytes.get(terminator) {
                    Some(b';') => {
                        declarations.push(Declaration {
                            name,
                            paths: path_values(&attributes),
                            semi: true,
                            start,
                            end: terminator + 1,
                        });
                        attributes.clear();
                        index = terminator + 1;
                    }
                    Some(b'{') => {
                        let end = balanced(text, terminator, b'{', b'}');
                        declarations.push(Declaration {
                            name,
                            paths: path_values(&attributes),
                            semi: false,
                            start,
                            end,
                        });
                        attributes.clear();
                        index = terminator + 1;
                    }
                    _ => {
                        attributes.clear();
                        index = next;
                    }
                }
            }
            "include" => {
                let after = trivia(text, next);
                if bytes.get(after) == Some(&b'!') {
                    let open = trivia(text, after + 1);
                    if bytes.get(open) == Some(&b'(') {
                        let end = balanced(text, open, b'(', b')');
                        if let Some((path, _)) = string_literal(text, trivia(text, open + 1)) {
                            includes.push(path);
                        }
                        attributes.clear();
                        index = end;
                        continue;
                    }
                }
                attributes.clear();
                index = next;
            }
            _ => {
                attributes.clear();
                index = next;
            }
        }
    }
    (declarations, includes)
}

fn path_values(attributes: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    for attribute in attributes {
        collect_paths(attribute, &mut values);
    }
    values
}

fn collect_paths(attribute: &str, values: &mut Vec<String>) {
    let bytes = attribute.as_bytes();
    let mut index = 0;
    while index < attribute.len() {
        index = trivia(attribute, index);
        if index >= attribute.len() {
            break;
        }
        if bytes[index] == b'"' {
            match string_literal(attribute, index) {
                Some((_, end)) => index = end,
                None => index = advance(attribute, index),
            }
        } else if ident_start(bytes[index]) {
            let (word, next) = ident(attribute, index);
            let after = trivia(attribute, next);
            if word == "path" && bytes.get(after) == Some(&b'=') {
                let value = trivia(attribute, after + 1);
                if let Some((literal, end)) = string_literal(attribute, value) {
                    values.push(literal);
                    index = end;
                    continue;
                }
            }
            index = next;
        } else {
            index = advance(attribute, index);
        }
    }
}
