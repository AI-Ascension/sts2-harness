// SPDX-License-Identifier: MIT
//! Tests for the exemption prose-count check and the exemption severity split.

use super::{exemption_count_findings, stated_line_count};
use crate::config::Policy;
use crate::diagnostic::{Finding, Severity};
use std::error::Error;
use std::path::{Path, PathBuf};

#[test]
fn reads_the_subject_count_and_not_the_stated_limit() {
    // Both numbers appear in this reason; only the subject form is the file's own size.
    assert_eq!(
        stated_line_count(
            "the changelog is archived by a wave; its 632 nonblank lines remain \
     below the 700-line markdown hard limit"
        ),
        Some(632)
    );
    assert_eq!(
        stated_line_count("its 168 nonblank lines remain below the 200-line workflow hard limit"),
        Some(168)
    );
    assert_eq!(
        stated_line_count("its 334-line implementation is below the hard limit"),
        Some(334)
    );
}

#[test]
fn a_reason_without_a_count_claims_nothing() {
    assert_eq!(
        stated_line_count("this module is reviewed as one bounded planning boundary"),
        None
    );
}

#[test]
fn a_limit_is_never_read_as_the_files_count() {
    // The `-line` form has no `the` asymmetry to protect it, so `below its 400-line
    // hard limit` would otherwise read 400 as the file's size and fail a compliant
    // file. Only a noun that describes the file makes the phrase a count.
    assert_eq!(
        stated_line_count(
            "the service keeps its ports together; this stays below its 400-line hard limit"
        ),
        None
    );
    assert_eq!(
        stated_line_count("this stays below its 600-line limit"),
        None
    );
    assert_eq!(
        stated_line_count("its 400-line maximum is not the size"),
        None
    );
    // The genuine count form still reads, even when a limit follows it.
    assert_eq!(
        stated_line_count("its 931-line implementation exceeds the 400-line hard limit"),
        Some(931)
    );
}

#[test]
fn an_unparseable_asserted_count_fails_closed() {
    // The sentence asserts a count but does not match the grammar. Reporting nothing
    // here would silently trust the number, which is the defect this check closes.
    assert_eq!(
        stated_line_count("its  lines remain below the 700-line markdown hard limit"),
        Some(0)
    );
    assert_eq!(
        stated_line_count("roughly 600 nonblank lines remain below the hard limit"),
        Some(0)
    );
}

/// Builds a real `Policy` through the production parser so the tests exercise the
/// same limit table the tool uses at runtime.
fn policy_with(rust: (usize, usize)) -> Result<Policy, Box<dyn Error>> {
    let text = format!(
        "policy_version = 1\n\
     [project]\n\
     required_files = []\n\
     ignored_directories = []\n\
     ignored_path_prefixes = []\n\
     [project.required_preambles]\n\
     [limits]\n\
     rust_production_preferred = {}\n\
     rust_production_max = {}\n\
     rust_test_preferred = 400\n\
     rust_test_max = 600\n\
     csharp_production_preferred = 300\n\
     csharp_production_max = 400\n\
     csharp_test_preferred = 400\n\
     csharp_test_max = 600\n\
     workflow_preferred = 150\n\
     workflow_max = 200\n\
     markdown_preferred = 500\n\
     markdown_max = 700\n\
     [exemptions]\n",
        rust.0, rust.1
    );
    Policy::parse(&text)
        .map_err(|error| -> Box<dyn Error> { Box::new(std::io::Error::other(error)) })
}

fn write(root: &Path, relative: &str, nonblank: usize) -> Result<(), Box<dyn Error>> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = (0..nonblank)
        .map(|index| format!("line {index}\n\n"))
        .collect::<String>();
    std::fs::write(path, text)?;
    Ok(())
}

fn temp_root(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    // Artifacts stay inside the worktree: /tmp is a 512 MiB tmpfs on this host and is
    // reserved for other lanes.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("exc-tests")
        .join(name);
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

#[test]
fn a_matching_count_produces_no_finding() -> Result<(), Box<dyn Error>> {
    let root = temp_root("match")?;
    write(&root, "crates/harness/src/lib.rs", 300)?;
    let policy = policy_with((300, 400))?;
    assert!(
        exemption_count_findings(
            &root,
            "crates/harness/src/lib.rs",
            "its 300 nonblank lines remain below the hard limit",
            &policy
        )
        .is_empty()
    );
    Ok(())
}

#[test]
fn stale_prose_on_a_compliant_file_is_a_warning() -> Result<(), Box<dyn Error>> {
    let root = temp_root("compliant")?;
    write(&root, "crates/harness/src/lib.rs", 270)?;
    let policy = policy_with((300, 400))?;
    let findings = exemption_count_findings(
        &root,
        "crates/harness/src/lib.rs",
        "its 334 nonblank lines remain below the hard limit",
        &policy,
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, Severity::Warning);
    assert!(findings[0].message.contains("334"));
    assert!(findings[0].message.contains("270"));
    Ok(())
}

#[test]
fn stale_prose_understating_a_hard_limit_breach_is_an_error() -> Result<(), Box<dyn Error>> {
    let root = temp_root("breach")?;
    write(&root, "crates/harness/src/lib.rs", 931)?;
    let policy = policy_with((300, 400))?;
    let findings = exemption_count_findings(
        &root,
        "crates/harness/src/lib.rs",
        "its 385 nonblank lines remain below the hard limit",
        &policy,
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, Severity::Error);
    assert_eq!(findings[0].rule, "EXC002");
    Ok(())
}

#[test]
fn an_unparseable_count_is_reported_rather_than_trusted() -> Result<(), Box<dyn Error>> {
    let root = temp_root("unparseable")?;
    write(&root, "crates/harness/src/lib.rs", 300)?;
    let policy = policy_with((300, 400))?;
    let findings = exemption_count_findings(
        &root,
        "crates/harness/src/lib.rs",
        "its  nonblank lines remain below the hard limit",
        &policy,
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, Severity::Error);
    assert!(findings[0].message.contains("does not parse"));
    Ok(())
}

#[test]
fn findings_carry_a_stable_rule_for_every_reported_case() -> Result<(), Box<dyn Error>> {
    let root = temp_root("rule")?;
    write(&root, "crates/harness/src/lib.rs", 500)?;
    let policy = policy_with((300, 400))?;
    let findings: Vec<Finding> = exemption_count_findings(
        &root,
        "crates/harness/src/lib.rs",
        "its 400 nonblank lines remain below the hard limit",
        &policy,
    );
    assert!(findings.iter().all(|finding| finding.rule == "EXC002"));
    Ok(())
}
