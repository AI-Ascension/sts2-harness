// SPDX-License-Identifier: MIT

//! Undecodable-manifest coverage for the `RUST002` reachability rule (#671).
//!
//! Split out of `modules_tests` for the reason `path_attribute_tests` is: both
//! files keep headroom under the `rust_test_preferred` budget. The rule under
//! test is that a `Cargo.toml` the tool cannot read is *reported* as `RUST003`
//! and excluded from the reachability check, rather than dropped in silence as
//! it was before #671.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use super::findings;
use super::{Fixture, Krate};
use crate::config::Policy;
use crate::files::collect;
use crate::module_roots::crates;

/// #671: a manifest carrying a single non-UTF-8 byte must be reported, not
/// dropped.
///
/// `crates()` used to `continue` on a failed decode, so the crate disappeared
/// from the graph entirely. `RUST002` then had nothing to evaluate for it and
/// reported nothing — a clean result that was really an unchecked crate.
///
/// Non-vacuity is proved by `pre_fix_crates_skips_the_bad_manifest` below, which
/// runs the pre-fix body verbatim and shows it returns nothing for this input.
#[test]
fn an_undecodable_manifest_is_reported() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    fs::create_dir_all(root.join("crate/src"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crate\"]\n",
    )?;
    // A comment carrying an invalid byte: legal-looking to a human, and a
    // decode failure to `read_to_string`, which is exactly the silent skip.
    fs::write(
        root.join("crate/Cargo.toml"),
        b"[package]\nname = \"demo\"\nversion = \"0.0.0\"\n# \xff\n",
    )?;
    fs::write(root.join("crate/src/lib.rs"), "")?;

    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let reported: Vec<(String, String)> = findings(root, &files)
        .iter()
        .map(|finding| (finding.rule.to_owned(), finding.path.clone()))
        .collect();
    assert_eq!(
        reported,
        vec![("RUST003".to_owned(), "crate/Cargo.toml".to_owned())],
        "the undecodable manifest must be reported exactly once, as RUST003"
    );
    Ok(())
}

/// The blast-radius guard from the issue: an undecodable manifest must not
/// turn into a wave of orphan findings.
///
/// This is why the fix *reports* rather than retains: retaining the crate with
/// an empty root set would let `owned()` claim the whole directory while nothing
/// in it is reachable, so every source file would be reported.
#[test]
fn an_undecodable_manifest_does_not_report_its_sources_as_orphans() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    fs::create_dir_all(root.join("crate/src/deep"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crate\"]\n",
    )?;
    fs::write(
        root.join("crate/Cargo.toml"),
        b"[package]\nname = \"demo\"\nversion = \"0.0.0\"\n# \xff\n",
    )?;
    fs::write(root.join("crate/src/lib.rs"), "mod deep;\n")?;
    fs::write(root.join("crate/src/deep/mod.rs"), "mod leaf;\n")?;
    fs::write(root.join("crate/src/deep/leaf.rs"), "")?;

    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let reported = findings(root, &files);
    let orphans: Vec<&str> = reported
        .iter()
        .filter(|finding| finding.rule == "RUST002")
        .map(|finding| finding.path.as_str())
        .collect();
    assert!(
        orphans.is_empty(),
        "an unreadable manifest must not orphan the files it hides: {orphans:?}"
    );
    assert_eq!(reported.len(), 1, "only the manifest itself is reported");
    Ok(())
}

/// The exclusion is per-crate, not per-run: a second, readable manifest in the
/// same tree is still resolved and still checked for orphans.
///
/// A blanket "return early on any unreadable manifest" would satisfy the two
/// tests above and still lose real coverage, so this one pins that the
/// remaining graph is still evaluated.
#[test]
fn one_undecodable_manifest_does_not_narrow_coverage_of_the_others() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    fs::create_dir_all(root.join("broken/src"))?;
    fs::create_dir_all(root.join("healthy/src"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"broken\", \"healthy\"]\n",
    )?;
    fs::write(
        root.join("broken/Cargo.toml"),
        b"[package]\nname = \"broken\"\nversion = \"0.0.0\"\n# \xff\n",
    )?;
    fs::write(root.join("broken/src/lib.rs"), "")?;
    fs::write(
        root.join("healthy/Cargo.toml"),
        "[package]\nname = \"healthy\"\nversion = \"0.0.0\"\n",
    )?;
    fs::write(root.join("healthy/src/lib.rs"), "mod kept;\n")?;
    fs::write(root.join("healthy/src/kept.rs"), "")?;
    // A genuine orphan in the *readable* crate: it must still be reported.
    fs::write(root.join("healthy/src/lost.rs"), "")?;

    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let reported = findings(root, &files);
    let orphans: Vec<&str> = reported
        .iter()
        .filter(|finding| finding.rule == "RUST002")
        .map(|finding| finding.path.as_str())
        .collect();
    assert_eq!(
        orphans,
        vec!["healthy/src/lost.rs"],
        "the readable crate must still be checked in full"
    );
    let manifests: Vec<&str> = reported
        .iter()
        .filter(|finding| finding.rule == "RUST003")
        .map(|finding| finding.path.as_str())
        .collect();
    assert_eq!(manifests, vec!["broken/Cargo.toml"]);
    Ok(())
}

/// A manifest that decodes but is not valid TOML is the same defect reached
/// from the other side, and is reported the same way.
#[test]
fn an_unparseable_manifest_is_reported() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    fs::create_dir_all(root.join("crate/src"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crate\"]\n",
    )?;
    fs::write(root.join("crate/Cargo.toml"), "[package\nname = \"demo\"\n")?;
    fs::write(root.join("crate/src/lib.rs"), "")?;

    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let reported = findings(root, &files);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0].rule, "RUST003");
    assert_eq!(reported[0].path, "crate/Cargo.toml");
    Ok(())
}

/// The control for the whole rule: on a tree where every manifest decodes, no
/// `RUST003` is raised. Without this, a fix that reported unconditionally would
/// satisfy every test above.
#[test]
fn a_fully_decodable_tree_raises_no_manifest_finding() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    fs::create_dir_all(root.join("crate/src"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crate\"]\n",
    )?;
    fs::write(
        root.join("crate/Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.0.0\"\n",
    )?;
    fs::write(root.join("crate/src/lib.rs"), "mod kept;\n")?;
    fs::write(root.join("crate/src/kept.rs"), "")?;

    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let reported = findings(root, &files);
    assert!(reported.is_empty(), "unexpected findings: {reported:?}");
    Ok(())
}

/// Non-vacuity: the pre-fix body, verbatim, is silent on the same input.
///
/// A new test that only asserts the new code reports something would also be
/// satisfied by a rule that reported unconditionally. This is the check the
/// campaign has been bitten by before, so it is asserted directly: `pre_fix`
/// below is the `crates()` loop exactly as it stood at `8848a58c` — the
/// `let Ok(text) = fs::read_to_string(manifest) else { continue }` skip and the
/// `Result`-free signature both included — and it returns an empty vector for
/// the same manifest the new code reports. If the old behaviour were already
/// loud, the new findings would not be a fix.
#[test]
fn pre_fix_crates_skips_the_bad_manifest() -> Result<(), Box<dyn Error>> {
    use std::collections::BTreeSet as Set;
    use std::fs;

    // The pre-fix implementation, transcribed from `module_roots.rs` at
    // `8848a58c` lines 23-51. Only the two `continue` arms under test are
    // relevant; the rest is unchanged so the comparison is like-for-like.
    fn pre_fix(root: &Path, files: &[PathBuf], sources: &Set<String>) -> Vec<Krate> {
        let mut crates = Vec::new();
        for manifest in files
            .iter()
            .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        {
            let Ok(text) = fs::read_to_string(manifest) else {
                continue;
            };
            let Ok(value) = toml::from_str::<toml::Value>(&text) else {
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
                .map(|parent| crate::files::relative_text(root, parent))
                .unwrap_or_default();
            let roots: Set<String> = [dir_join(&dir, "src/lib.rs"), dir_join(&dir, "src/main.rs")]
                .into_iter()
                .filter(|candidate| sources.contains(candidate))
                .collect();
            if !roots.is_empty() {
                crates.push(Krate { dir, roots });
            }
        }
        crates
    }

    fn dir_join(dir: &str, relative: &str) -> String {
        if dir.is_empty() {
            relative.to_owned()
        } else {
            format!("{dir}/{relative}")
        }
    }

    let fixture = Fixture::new()?;
    let root = &fixture.0;
    fs::create_dir_all(root.join("crate/src"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crate\"]\n",
    )?;
    fs::write(
        root.join("crate/Cargo.toml"),
        b"[package]\nname = \"demo\"\nversion = \"0.0.0\"\n# \xff\n",
    )?;
    fs::write(root.join("crate/src/lib.rs"), "")?;

    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let sources: Set<String> = files
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| crate::files::relative_text(root, path))
        .collect();

    let (resolved, unreadable) = crates(root, &files, &sources);
    assert_eq!(unreadable.len(), 1, "the new code must report the manifest");
    assert!(
        resolved.is_empty(),
        "the unreadable manifest must not enter the graph"
    );
    assert!(
        pre_fix(root, &files, &sources).is_empty(),
        "PRE-FIX LOGIC WAS NOT SILENT: the bad manifest resolved to a crate, so \
         the new finding would not be a fix"
    );
    Ok(())
}
