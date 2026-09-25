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
            // An `include!`d file is compiled in place, so its own directory,
            // not the includer's, is the base for a `mod child;` written in it.
            let fragment_dir = directory(&target);
            enqueue(&mut pending, sources, target, fragment_dir);
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
    let states = bases(file, child_dir, declarations, declaration.start);
    if !declaration.paths.is_empty() {
        // `#[path]` on an inline module names the directory its children live in;
        // rustc reads no file there, so the module owns nothing itself and the
        // value is only a prefix. On a semicolon module it always names a file:
        // a directory value is a rustc error (`Is a directory`), not a missed
        // reachability, so `DIR/mod.rs` is deliberately not consulted.
        //
        // A `#[cfg_attr]`-gated `#[path]` replaces the name-based lookup only on
        // the branch that takes it, so when the attribute is conditional the
        // ordinary child lookup below still runs: under the default build rustc
        // reads `src/m/child.rs` for `#[cfg_attr(feature = "x", path = "alt")]
        // mod m { pub mod child; }`, and neither branch may be reported.
        if !declaration.semi {
            return Vec::new();
        }
        let mut targets: Vec<(String, String)> = states
            .iter()
            .flat_map(|state| {
                let base = state.base(file);
                let paths = declaration.paths.clone();
                paths.into_iter().map(move |path| {
                    let target = join(&base, &path);
                    let directory = directory(&target);
                    (target, directory)
                })
            })
            .collect();
        // A `#[cfg_attr]`-gated `mod NAME;` keeps the ordinary lookup as well:
        // `src/m.rs` under the default build and `src/alt.rs` with the gate
        // taken are each real on their own branch, so both are credited.
        if !declaration.conditional {
            return targets;
        }
        targets.extend(
            states
                .into_iter()
                .flat_map(|state| ordinary_children(&state, declaration)),
        );
        return targets;
    }
    if !declaration.semi {
        return Vec::new();
    }
    states
        .into_iter()
        .flat_map(|state| ordinary_children(&state, declaration))
        .collect()
}

/// The ordinary `NAME.rs` / `NAME/mod.rs` lookup for a `mod NAME;` declaration,
/// resolved inside the directory its enclosing blocks contributed.
fn ordinary_children(state: &Base, declaration: &Declaration) -> Vec<(String, String)> {
    let dir = state.children_dir();
    let children = join(&dir, &declaration.name);
    let stem = join(&dir, &format!("{}.rs", declaration.name));
    let nested = join(&children, "mod.rs");
    [(stem, children.clone()), (nested, children)]
        .into_iter()
        .collect()
}

/// The `#[cfg_attr]`-gated `#[path]` targets, which resolve against the same
/// base the ordinary lookup uses.
/// Where a declaration's own resolution starts, one entry per `#[path]` branch.
struct Base {
    dir: String,
    pending: Vec<String>,
    anchored: bool,
}

impl Base {
    /// The directory a `#[path]` value is relative to. An unnested declaration
    /// resolves against the directory of the file that carries it — that is
    /// `src/` for `src/x.rs`, not the `src/x/` its ordinary children use.
    fn base(&self, file: &str) -> String {
        if self.anchored || !self.pending.is_empty() {
            join_all(&self.dir, &self.pending)
        } else {
            directory(file)
        }
    }

    /// The directory an ordinary `mod name;` child resolves in.
    fn children_dir(&self) -> String {
        join_all(&self.dir, &self.pending)
    }
}

/// The state of every inline `mod` block enclosing `position`, outermost first.
///
/// A plain inline module nests one directory deeper, while one carrying a
/// `#[path]` names its children's directory outright: its own name is dropped
/// and any enclosing names are superseded, because rustc resolves that path
/// against the directory holding the *file*, then treats it as the module's
/// directory. Each `#[cfg_attr]` branch is a separate state, since each is a
/// real directory on the platform that selects it.
fn bases(file: &str, child_dir: &str, declarations: &[Declaration], position: usize) -> Vec<Base> {
    let mut enclosing: Vec<&Declaration> = declarations
        .iter()
        .filter(|declaration| {
            !declaration.semi && declaration.start < position && position < declaration.end
        })
        .collect();
    enclosing.sort_by_key(|declaration| declaration.start);
    let mut states = vec![Base {
        dir: child_dir.to_owned(),
        pending: Vec::new(),
        anchored: false,
    }];
    for ancestor in enclosing {
        states = states
            .into_iter()
            .flat_map(|state| {
                // A conditional block keeps the name-based state *as well as*
                // its `#[path]` states: the path only replaces the name on the
                // branch that takes it, so a child written in the block is
                // reachable under both directories.
                let base = state.base(file);
                let mut branches = Vec::new();
                if ancestor.paths.is_empty() || ancestor.conditional {
                    let mut nested = state.pending;
                    nested.push(ancestor.name.clone());
                    branches.push(Base {
                        dir: state.dir,
                        pending: nested,
                        anchored: state.anchored,
                    });
                }
                if ancestor.paths.is_empty() {
                    return branches;
                }
                branches.extend(ancestor.paths.iter().map(|path| Base {
                    dir: join(&base, path),
                    pending: Vec::new(),
                    // Anchored: the inner `#[path]` is relative to the
                    // directory the outer one named, not to the carrying
                    // file. `inline_path_attribute_inside_a_path_module_is_anchored`
                    // is the only shape that reaches this flag.
                    anchored: true,
                }));
                branches
            })
            .collect();
    }
    states
}

#[cfg(test)]
#[path = "modules_tests.rs"]
mod tests;
