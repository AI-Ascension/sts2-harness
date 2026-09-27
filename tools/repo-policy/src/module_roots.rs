// SPDX-License-Identifier: MIT

//! Crate-root resolution for `RUST002`: which files Cargo/rustc treats as roots.
//!
//! Roots come from the manifest (`[lib]`, `[[bin]]`, `[[test]]`, `[[bench]]`,
//! `[[example]]`, `build.rs`) and from Cargo's auto-discovery conventions
//! (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`, `src/bin/<name>/main.rs`,
//! `tests|benches|examples/*.rs`).
//!
//! A manifest that cannot be decoded or cannot be parsed is **reported**, not
//! skipped. Dropping it made the crate vanish from the graph entirely, so
//! `RUST002` never evaluated it and the coverage it appeared to provide was
//! never real. A crate whose roots are unknown cannot be checked, and an
//! unchecked crate is a silent gap rather than a clean pass.
//!
//! ## Why the crate is reported and excluded, not retained
//!
//! The obvious alternative — keep the crate in the graph with an empty root
//! set — is strictly worse. `RUST002` decides orphans by `owned()`, which
//! claims every file under a crate's directory. A crate with no roots would
//! own its whole directory while *nothing* in it is reachable, so a single
//! stray byte would be reported as an orphan for every source file the crate
//! holds. That converts one honest coverage gap into a wave of false
//! accusations, and a false accusation is the more expensive error for this
//! rule: a missed orphan costs one lost `mod` line, while a spurious one
//! points at a live file.
//!
//! Reporting is also the consistent choice within this tool. `RUST001` already
//! reports "cannot parse Rust configuration" rather than dropping the file, and
//! #666 (`38ca3f9e`) removed exactly this species of silent decode skip for the
//! size and conflict rules. Silence was the defect; an explicit error is the
//! fix.
//!
//! The exclusion is *per-crate*, not per-run. Every manifest that does parse is
//! still resolved and checked, so an unreadable manifest costs its own crate
//! its reachability check and costs the rest of the tree nothing.
//!
//! There is no byte-level fallback, unlike #666's size and conflict fixes. Those
//! ask a question bytes can answer (how many nonblank lines, does this line
//! match a marker). This asks a TOML *parse*, which has no meaningful
//! approximation: guessing a crate's roots from undecodable bytes would
//! invent a module graph, and a wrong graph produces wrong orphans in both
//! directions. Reporting the unresolvable manifest is the honest outcome.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::byte_scan::read_bytes;
use crate::diagnostic::Finding;
use crate::files::relative_text;

pub(crate) struct Krate {
    pub(crate) dir: String,
    pub(crate) roots: BTreeSet<String>,
}

/// A manifest present in the tree that `RUST002` could not turn into roots.
struct Unreadable {
    path: String,
    reason: String,
}

/// The rule id for an unreadable manifest. Distinct from `RUST002` because
/// "this crate could not be checked" and "this file is unreachable" are
/// different facts; conflating them is the ambiguity that hid #671.
const MANIFEST_RULE: &str = "RUST003";
const MANIFEST_MESSAGE: &str =
    "cannot read the crate manifest, so its module reachability was not checked: ";

/// Resolves every crate in the tree.
///
/// The second element carries one `RUST003` finding per manifest that could not
/// be decoded or parsed, in path order. Such a manifest is *not* treated as a
/// crate with no roots, because such a crate would own its whole directory and
/// report every file in it as an orphan — a coverage gap would become a wave of
/// false accusations instead of one honest error.
///
/// The two are separate rather than a `Result` because the failure is
/// per-manifest, not per-run: one unreadable manifest must not cost the tree the
/// reachability check on every crate that *did* parse. A `Result` would discard
/// the resolved crates along with the complaint.
pub(crate) fn crates(
    root: &Path,
    files: &[PathBuf],
    sources: &BTreeSet<String>,
) -> (Vec<Krate>, Vec<Finding>) {
    let mut crates = Vec::new();
    let mut unreadable = Vec::new();
    for manifest in files
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
    {
        let relative = relative_text(root, manifest);
        // Bytes, not a decoded read: a single invalid byte must reach the
        // reporter as a fact rather than disappearing into a failed decode.
        // There is no byte-level fallback for a manifest, because the decision
        // being made is a TOML *parse*, not a line count or a pattern match.
        let Some(bytes) = read_bytes(manifest) else {
            unreadable.push(Unreadable {
                path: relative,
                reason: "cannot read manifest".to_owned(),
            });
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            unreadable.push(Unreadable {
                path: relative,
                reason: "manifest is not valid UTF-8".to_owned(),
            });
            continue;
        };
        let Ok(value) = toml::from_str::<Value>(text) else {
            unreadable.push(Unreadable {
                path: relative,
                reason: "manifest is not valid TOML".to_owned(),
            });
            continue;
        };
        if value.get("package").is_none()
            && value.get("lib").is_none()
            && value.get("bin").is_none()
        {
            continue;
        }
        let dir = manifest
            .parent()
            .map(|parent| relative_text(root, parent))
            .unwrap_or_default();
        let roots = roots(&dir, &value, sources);
        if !roots.is_empty() {
            crates.push(Krate { dir, roots });
        }
    }
    let findings = unreadable
        .iter()
        .map(|Unreadable { path, reason }| {
            Finding::error(MANIFEST_RULE, path, format!("{MANIFEST_MESSAGE}{reason}"))
        })
        .collect();
    (crates, findings)
}

fn roots(dir: &str, manifest: &Value, sources: &BTreeSet<String>) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    match manifest
        .get("lib")
        .and_then(|lib| lib.get("path"))
        .and_then(Value::as_str)
    {
        Some(path) => add(dir, sources, &mut roots, path),
        None => add(dir, sources, &mut roots, "src/lib.rs"),
    }
    if manifest
        .get("package")
        .and_then(|package| package.get("build"))
        != Some(&Value::Boolean(false))
    {
        add(dir, sources, &mut roots, "build.rs");
    }
    for entry in array(manifest, "bin") {
        if let Some(path) = entry.get("path").and_then(Value::as_str) {
            add(dir, sources, &mut roots, path);
        } else if let Some(name) = entry.get("name").and_then(Value::as_str) {
            add(dir, sources, &mut roots, &format!("src/bin/{name}.rs"));
        }
    }
    add(dir, sources, &mut roots, "src/main.rs");
    roots.extend(discovered(dir, sources, "src/bin"));
    for (kind, directory) in [
        ("test", "tests"),
        ("bench", "benches"),
        ("example", "examples"),
    ] {
        for entry in array(manifest, kind) {
            if let Some(path) = entry.get("path").and_then(Value::as_str) {
                add(dir, sources, &mut roots, path);
            }
        }
        roots.extend(discovered(dir, sources, directory));
    }
    roots
}

fn add(dir: &str, sources: &BTreeSet<String>, roots: &mut BTreeSet<String>, relative: &str) {
    let candidate = join(dir, relative);
    if sources.contains(&candidate) {
        roots.insert(candidate);
    }
}

fn array<'a>(manifest: &'a Value, key: &str) -> &'a [Value] {
    manifest
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Cargo's auto-discovery: direct `subdir/*.rs`, plus `src/bin/<name>/main.rs`.
fn discovered(dir: &str, sources: &BTreeSet<String>, subdir: &str) -> Vec<String> {
    let prefix = join(dir, subdir) + "/";
    sources
        .iter()
        .filter(|source| {
            let Some(rest) = source.strip_prefix(&prefix) else {
                return false;
            };
            match subdir {
                "src/bin" => match rest.split('/').collect::<Vec<_>>().as_slice() {
                    [file] => file.ends_with(".rs"),
                    [name, "main.rs"] => !name.is_empty(),
                    _ => false,
                },
                _ => !rest.contains('/') && rest.ends_with(".rs"),
            }
        })
        .cloned()
        .collect()
}

fn join(dir: &str, relative: &str) -> String {
    if dir.is_empty() {
        normalise(relative)
    } else {
        normalise(&format!("{dir}/{relative}"))
    }
}

fn normalise(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}
