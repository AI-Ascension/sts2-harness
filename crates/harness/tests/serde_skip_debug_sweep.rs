// SPDX-License-Identifier: MIT

//! Issue #247 AC1 acceptance guard.
//!
//! `#[serde(skip)]` governs serialization only. A type that derives `Debug` while carrying such a
//! field prints the field the author deliberately kept off the wire, which is how #241, #243 and
//! #247 all leaked private bytes. #247 requires that no type in `crates/harness/src` both derives
//! `Debug` and carries a `#[serde(skip)]` field **unless the deviation is justified inline**.
//!
//! This test is the executable form of that rule. It scans the real sources, so the sweep stays
//! exhaustive as the workspace grows instead of decaying into a one-off manual audit, and it fails
//! loudly when a new derived-`Debug`/`serde(skip)` combination appears without a justification.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The one accepted deviation, with the exact justification token its source comment must carry.
///
/// `MemoryQuery.query_id` is a bounded correlation id (`valid_id` caps it at 128 ASCII
/// alphanumeric/`._:-` bytes), never private content bytes, and `MemoryCorpus::retrieve` copies it
/// into the serialized `RetrievalResponse.query_id`, so it is not withheld from observers. It is
/// skipped from `query.v1` only so a returned response cannot be mistaken for an input.
const JUSTIFIED: &[(&str, &str, &str, &str)] = &[(
    "crates/harness/src/context_memory/retrieval_types.rs",
    "MemoryQuery",
    "query_id",
    "#247 AC1",
)];

struct Site {
    file: String,
    line: usize,
    type_name: String,
    field: String,
    justified: bool,
}

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|value| value == "rs") {
            out.push(path);
        }
    }
}

/// `#[serde(skip)]` verbatim. `skip_serializing_if`, `skip_serializing` and `skip_deserializing`
/// are conditional wire selections, not secrecy markers, so they are deliberately not matched.
fn is_exact_skip(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("#[serde(") && line.ends_with(")]") && {
        let inner = &line["#[serde(".len()..line.len() - ")]".len()];
        inner.split(',').any(|item| item.trim() == "skip")
    }
}

/// The contiguous attribute/comment block immediately above a declaration, joined for substring
/// matching so a multi-line `#[derive(...)]` is still read as a unit.
fn attributes_above(lines: &[&str], index: usize) -> String {
    let mut attributes = Vec::new();
    let mut cursor = index;
    while cursor > 0 {
        let line = lines[cursor - 1].trim();
        if line.starts_with("#[") || line.starts_with("//") || line.starts_with("///") {
            attributes.push(line);
            cursor -= 1;
        } else if line.is_empty() {
            cursor -= 1;
        } else {
            break;
        }
    }
    attributes.join("\n")
}

/// Exclusive end index of a declaration whose body opens at `start`, by brace balance.
fn body_end(lines: &[&str], start: usize) -> usize {
    let mut depth: i64 = 0;
    let mut opened = false;
    for (offset, line) in lines[start..].iter().enumerate() {
        depth += line.matches('{').count() as i64;
        depth -= line.matches('}').count() as i64;
        if line.contains('{') {
            opened = true;
        }
        if opened && depth <= 0 {
            return start + offset + 1;
        }
    }
    lines.len()
}

fn field_name(line: &str) -> Option<String> {
    let (name, _) = line.trim().split_once(':')?;
    let name = name.trim();
    let name = name.strip_prefix("pub ").unwrap_or(name);
    let name = name.strip_prefix("pub(crate) ").unwrap_or(name);
    name.chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
        .then(|| name.to_owned())
}

/// Collect every `struct`/`enum` that derives `Debug` and carries an exact `#[serde(skip)]` field.
fn sites() -> Vec<Site> {
    let mut files = Vec::new();
    rust_sources(&source_root(), &mut files);
    files.sort();

    let mut found = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(source_root())
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let relative = format!("crates/harness/src/{relative}");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let rest = trimmed
                .strip_prefix("pub struct ")
                .or_else(|| trimmed.strip_prefix("struct "))
                .or_else(|| trimmed.strip_prefix("pub enum "))
                .or_else(|| trimmed.strip_prefix("enum "));
            let Some(rest) = rest else {
                continue;
            };
            let Some((type_name, _)) = rest.split_once(|character: char| {
                character == '{' || character == '<' || character.is_whitespace()
            }) else {
                continue;
            };
            let attributes = attributes_above(&lines, index);
            let derives_debug = attributes.contains("#[derive") && attributes.contains("Debug");
            if !derives_debug {
                continue;
            }
            let end = body_end(&lines, index);
            for (offset, body_line) in lines[index..end].iter().enumerate() {
                if !is_exact_skip(body_line) {
                    continue;
                }
                let Some(field) = lines[index + offset + 1..end]
                    .first()
                    .and_then(|line| field_name(line))
                else {
                    continue;
                };
                // Justification is documented immediately above the derive on the type.
                let justified = attributes.contains("#247 AC1");
                found.push(Site {
                    file: relative.clone(),
                    line: index + 1,
                    type_name: type_name.to_owned(),
                    field,
                    justified,
                });
            }
        }
    }
    found
}

#[test]
fn every_derived_debug_over_serde_skip_is_fixed_or_justified_inline() {
    let found = sites();
    let mut unexpected = Vec::new();
    let mut matched_justifications = BTreeSet::new();

    for site in &found {
        let justification = JUSTIFIED.iter().find(|(file, type_name, field, _)| {
            site.file == *file
                && site.type_name == *type_name
                && site.field == *field
                && site.justified
        });
        match justification {
            Some((_, _, _, token)) => {
                matched_justifications.insert((*token).to_owned());
            }
            None => unexpected.push(format!(
                "{}:{} {}::{}",
                site.file, site.line, site.type_name, site.field
            )),
        }
    }

    assert!(
        unexpected.is_empty(),
        "these types derive `Debug` over an exact `#[serde(skip)]` field without an inline \
         justification, so `{{:?}}` can publish a field the author kept off the wire: {unexpected:#?}"
    );

    // Guards the guard: if the accepted deviation is ever fixed properly, or the scan silently
    // stops matching it, this fails so the allowlist cannot rot into a blanket exemption.
    assert_eq!(
        matched_justifications.len(),
        JUSTIFIED.len(),
        "the accepted deviation must still be found and justify itself inline"
    );
}
