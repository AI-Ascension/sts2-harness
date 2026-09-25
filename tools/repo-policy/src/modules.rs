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

use crate::diagnostic::Finding;
use crate::files::relative_text;
use crate::module_paths::{directory, join, join_all};
use crate::module_roots::{Krate, crates};
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

fn contents(
    root: &Path,
    files: &[PathBuf],
    sources: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut contents = BTreeMap::new();
    for path in files {
        let relative = relative_text(root, path);
        if sources.contains(&relative)
            && let Ok(text) = fs::read_to_string(path)
        {
            contents.insert(relative, text);
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
        (
            join(&dir, &format!("{}.rs", declaration.name)),
            children.clone(),
        ),
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

#[cfg(test)]
#[path = "modules_tests.rs"]
mod tests;
