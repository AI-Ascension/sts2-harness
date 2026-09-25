// SPDX-License-Identifier: MIT

//! Unit and control tests for the `RUST002` module-reachability rule.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{Krate, findings, orphans};
use crate::config::Policy;
use crate::files::collect;

/// Builds an in-memory tree whose single crate owns every file, then returns the
/// files the rule reports as unreachable.
fn unreachable(files: &[(&str, &str)], roots: &[&str]) -> BTreeSet<String> {
    let sources: BTreeSet<String> = files.iter().map(|(path, _)| (*path).to_owned()).collect();
    let contents: BTreeMap<String, String> = files
        .iter()
        .map(|(path, body)| ((*path).to_owned(), (*body).to_owned()))
        .collect();
    let krate = Krate {
        dir: String::new(),
        roots: roots.iter().map(|root| (*root).to_owned()).collect(),
    };
    orphans(&sources, &contents, &[krate])
}

#[test]
fn flags_a_lost_declaration() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "mod kept;\n"),
            ("src/kept.rs", ""),
            ("src/lost.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(reported, BTreeSet::from(["src/lost.rs".to_owned()]));
}

#[test]
fn resolves_flat_files_and_mod_rs_children() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "mod flat;\nmod nested;\n"),
            ("src/flat.rs", ""),
            ("src/nested/mod.rs", "mod child;\n"),
            ("src/nested/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(reported.is_empty(), "{reported:?}");
}

#[test]
fn path_attribute_owns_its_own_directory() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "#[path = \"sub/foo.rs\"]\nmod foo;\n"),
            ("src/sub/foo.rs", "mod sibling;\n"),
            ("src/sub/sibling.rs", ""),
            ("src/sub/foo/sibling.rs", ""),
        ],
        &["src/lib.rs"],
    );
    // A `#[path]` file looks for children in its own directory, not a stem
    // subdirectory, so the stem-shaped copy is the orphan.
    assert_eq!(
        reported,
        BTreeSet::from(["src/sub/foo/sibling.rs".to_owned()])
    );
}

#[test]
fn cfg_attr_path_reaches_every_branch() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(unix, path = \"a.rs\")]\n\
                 #[cfg_attr(not(unix), path = \"b.rs\")]\nmod gated;\n",
            ),
            ("src/a.rs", ""),
            ("src/b.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(reported.is_empty(), "{reported:?}");
}

#[test]
fn include_reaches_source_and_its_children() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "include!(\"shared.rs\");\n"),
            ("src/shared.rs", "mod extra;\n"),
            ("src/extra.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(reported.is_empty(), "{reported:?}");
}

#[test]
fn inline_module_owns_a_directory() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "mod outer {\n    mod inner;\n}\n"),
            ("src/outer/inner.rs", ""),
            ("src/inner.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(reported, BTreeSet::from(["src/inner.rs".to_owned()]));
}

/// `mod r#move;` is legal and common where the module name is a keyword, and
/// rustc resolves it to `move.rs` — `rustc` on `mod r#move;` with only
/// `r#move.rs` present fails `E0583: file not found for module \`r#move\`` and
/// names `src/move.rs` as the file to create. Reading the declaration as the
/// literal text `r#move` would report a file rustc compiles.
#[test]
fn raw_identifier_module_resolves_to_the_bare_stem() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "mod r#move;\n"),
            ("src/move.rs", ""),
            ("src/r#move.rs", ""),
        ],
        &["src/lib.rs"],
    );
    // The `r#`-prefixed filename is the orphan: rustc never loads it.
    assert_eq!(reported, BTreeSet::from(["src/r#move.rs".to_owned()]));
}

/// An inline raw-identifier module scopes its children under the bare name:
/// `mod r#type { mod child; }` in `src/lib.rs` resolves `child` in `src/type/`,
/// not `src/r#type/` (verified with `rustc 1.97.1`: the prefixed directory fails
/// `E0583`).
#[test]
fn inline_raw_identifier_module_owns_its_bare_directory() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "mod r#type {\n    mod child;\n}\n"),
            ("src/type/child.rs", ""),
            ("src/r#type/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(reported, BTreeSet::from(["src/r#type/child.rs".to_owned()]));
}

#[test]
fn ignores_declarations_inside_comments_and_strings() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "// mod comment_only;\nconst TEXT: &str = \"mod string_only;\";\nmod real;\n",
            ),
            ("src/real.rs", ""),
            ("src/comment_only.rs", ""),
            ("src/string_only.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(
        reported,
        BTreeSet::from([
            "src/comment_only.rs".to_owned(),
            "src/string_only.rs".to_owned()
        ])
    );
}

#[test]
fn escaped_character_and_raw_literals_do_not_hide_later_declarations() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "const QUOTE: char = '\\'';\n\
                 const RAW: &str = r#\"mod raw_hidden;\"#;\n\
                 mod after;\n",
            ),
            ("src/after.rs", ""),
            ("src/raw_hidden.rs", ""),
        ],
        &["src/lib.rs"],
    );
    // The escaped quote must not swallow `mod after;`, and the raw-string
    // contents must not invent `raw_hidden`.
    assert_eq!(reported, BTreeSet::from(["src/raw_hidden.rs".to_owned()]));
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "repo-policy-modules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

/// A real directory scan: reports a genuine orphan and spare files while leaving
/// ordinary roots, `#[cfg_attr]` targets, a `[[bin]] path` target, and a
/// `rustc`-standalone file alone.
#[test]
fn scans_a_crate_on_disk_with_controls() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    fs::create_dir_all(root.join("crate/src"))?;
    fs::create_dir_all(root.join("spike"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crate\"]\n",
    )?;
    fs::write(
        root.join("crate/Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.0.0\"\n\
         [[bin]]\nname = \"tool\"\npath = \"src/tool_main.rs\"\n",
    )?;
    fs::write(
        root.join("crate/src/lib.rs"),
        "mod reachable;\ninclude!(\"included.rs\");\n\
         #[cfg_attr(unix, path = \"dual_unix.rs\")]\n\
         #[cfg_attr(not(unix), path = \"dual_other.rs\")]\nmod dual;\n",
    )?;
    for name in [
        "reachable.rs",
        "included.rs",
        "dual_unix.rs",
        "dual_other.rs",
        "tool_main.rs",
        "orphan.rs",
    ] {
        fs::write(root.join(format!("crate/src/{name}")), "")?;
    }
    fs::write(root.join("spike/standalone.rs"), "")?;
    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let reported: BTreeSet<String> = findings(root, &files)
        .iter()
        .map(|finding| finding.path.clone())
        .collect();
    assert_eq!(reported, BTreeSet::from(["crate/src/orphan.rs".to_owned()]));
    Ok(())
}

#[test]
fn real_repository_controls_are_reachable() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let policy = Policy::load(&root.join("policy.toml"))?;
    let files = collect(&root, &policy)?;
    let reported: BTreeSet<String> = findings(&root, &files)
        .into_iter()
        .map(|finding| finding.path)
        .collect();
    for control in [
        "crates/harness/src/recorded_run_snapshot.rs",
        "crates/harness/src/recorded_run_snapshot_unsupported.rs",
        "tools/exact-restore-conformance/src/main.rs",
    ] {
        assert!(!reported.contains(control), "{control} was reported");
    }
    assert!(
        !reported
            .iter()
            .any(|path| path.contains("synthetic_model.rs")),
        "standalone fixture was reported"
    );
    assert!(reported.is_empty(), "unexpected orphans: {reported:?}");
    Ok(())
}
