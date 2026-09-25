// SPDX-License-Identifier: MIT

//! `RUST002`: every tracked Rust source must be reachable from a crate root.
//!
//! rustc compiles only the files a crate root reaches through `mod`, `#[path]`,
//! or `include!`. A lost declaration leaves a file in the tree that never
//! compiles while still looking like live code, so its tests silently stop
//! running. This rule reproduces rustc's file resolution: `mod x;` looks in
//! `DIR/x.rs` then `DIR/x/mod.rs`; a `#[path]` file keeps its children in the
//! file's own directory; crate roots, each `[[bin]]` path, and `mod.rs` child
//! lookup are honoured. `cfg` and `cfg_attr` gates are treated as always taken,
//! so only a file reachable from no target under any gate is reported.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::diagnostic::Finding;
use crate::files::relative_text;
use crate::module_scan::{Declaration, scan};

const RULE: &str = "RUST002";
const MESSAGE: &str =
    "Rust source is not reachable from any crate root; restore its `mod` declaration or delete it";

pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    if !root.join("Cargo.toml").is_file() {
        return Vec::new();
    }
    let sources: BTreeSet<String> = files
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| relative_text(root, path))
        .collect();
    let crates = crates(root, files, &sources);
    if crates.is_empty() {
        return Vec::new();
    }
    let contents = contents(root, files, &sources);
    orphans(&sources, &contents, &crates)
        .into_iter()
        .map(|source| Finding::error(RULE, &source, MESSAGE))
        .collect()
}

struct Krate {
    dir: String,
    roots: BTreeSet<String>,
}

fn crates(root: &Path, files: &[PathBuf], sources: &BTreeSet<String>) -> Vec<Krate> {
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
        if value.get("package").is_none() && value.get("lib").is_none() && value.get("bin").is_none()
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
    if manifest.get("package").and_then(|package| package.get("build")) != Some(&Value::Boolean(false))
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

fn contents(
    root: &Path,
    files: &[PathBuf],
    sources: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut contents = BTreeMap::new();
    for path in files {
        let relative = relative_text(root, path);
        if sources.contains(&relative) {
            if let Ok(text) = fs::read_to_string(path) {
                contents.insert(relative, text);
            }
        }
    }
    contents
}

fn orphans(
    sources: &BTreeSet<String>,
    contents: &BTreeMap<String, String>,
    crates: &[Krate],
) -> BTreeSet<String> {
    let reachable = reachable(sources, contents, crates);
    sources
        .iter()
        .filter(|source| owned(crates, source) && !reachable.contains(*source))
        .cloned()
        .collect()
}

fn owned(crates: &[Krate], source: &str) -> bool {
    crates
        .iter()
        .any(|krate| krate.dir.is_empty() || source.starts_with(&format!("{}/", krate.dir)))
}

fn reachable(
    sources: &BTreeSet<String>,
    contents: &BTreeMap<String, String>,
    crates: &[Krate],
) -> BTreeSet<String> {
    let mut pending: VecDeque<(String, String)> = VecDeque::new();
    for krate in crates {
        for root in &krate.roots {
            pending.push_back((root.clone(), directory(root)));
        }
    }
    let mut reached = BTreeSet::new();
    let mut seen = BTreeSet::new();
    while let Some((file, child_dir)) = pending.pop_front() {
        if !seen.insert((file.clone(), child_dir.clone())) {
            continue;
        }
        reached.insert(file.clone());
        let Some(text) = contents.get(&file) else {
            continue;
        };
        let (declarations, includes) = scan(text);
        for declaration in &declarations {
            for (target, directory) in targets(&file, &child_dir, declaration, &declarations) {
                enqueue(&mut pending, sources, target, directory);
            }
        }
        for include in &includes {
            let target = join(&directory(&file), include);
            enqueue(&mut pending, sources, target, child_dir.clone());
        }
    }
    reached
}

fn enqueue(
    pending: &mut VecDeque<(String, String)>,
    sources: &BTreeSet<String>,
    target: String,
    child_dir: String,
) {
    if sources.contains(&target) {
        pending.push_back((target, child_dir));
    }
}

fn targets(
    file: &str,
    child_dir: &str,
    declaration: &Declaration,
    declarations: &[Declaration],
) -> Vec<(String, String)> {
    let components = enclosing(declarations, declaration.start);
    if !declaration.paths.is_empty() {
        let base = if components.is_empty() {
            directory(file)
        } else {
            join_all(child_dir, &components)
        };
        return declaration
            .paths
            .iter()
            .map(|path| {
                let target = join(&base, path);
                let directory = directory(&target);
                (target, directory)
            })
            .collect();
    }
    if !declaration.semi {
        return Vec::new();
    }
    let dir = join_all(child_dir, &components);
    let children = join(&dir, &declaration.name);
    vec![
        (join(&dir, &format!("{}.rs", declaration.name)), children.clone()),
        (join(&children, "mod.rs"), children),
    ]
}

/// Inline `mod name { }` blocks that textually contain `position`, outermost first.
fn enclosing(declarations: &[Declaration], position: usize) -> Vec<String> {
    let mut components: Vec<(usize, String)> = declarations
        .iter()
        .filter(|declaration| {
            !declaration.semi && declaration.start < position && position < declaration.end
        })
        .map(|declaration| (declaration.start, declaration.name.clone()))
        .collect();
    components.sort_by_key(|(start, _)| *start);
    components.into_iter().map(|(_, name)| name).collect()
}

fn directory(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_owned(),
        None => String::new(),
    }
}

fn join(dir: &str, relative: &str) -> String {
    if dir.is_empty() {
        normalise(relative)
    } else {
        normalise(&format!("{dir}/{relative}"))
    }
}

fn join_all(dir: &str, parts: &[String]) -> String {
    let mut joined = dir.to_owned();
    for part in parts {
        joined = join(&joined, part);
    }
    joined
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

#[cfg(test)]
#[path = "modules_tests.rs"]
mod tests;
