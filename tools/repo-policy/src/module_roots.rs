// SPDX-License-Identifier: MIT

//! Crate-root resolution for `RUST002`: which files Cargo/rustc treats as roots.
//!
//! Roots come from the manifest (`[lib]`, `[[bin]]`, `[[test]]`, `[[bench]]`,
//! `[[example]]`, `build.rs`) and from Cargo's auto-discovery conventions
//! (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`, `src/bin/<name>/main.rs`,
//! `tests|benches|examples/*.rs`).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::files::relative_text;

pub(crate) struct Krate {
    pub(crate) dir: String,
    pub(crate) roots: BTreeSet<String>,
}

pub(crate) fn crates(root: &Path, files: &[PathBuf], sources: &BTreeSet<String>) -> Vec<Krate> {
    let mut crates = Vec::new();
    for manifest in files
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
    {
        let Ok(text) = fs::read_to_string(manifest) else {
            continue;
        };
        let Ok(value) = toml::from_str::<Value>(&text) else {
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
    crates
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
