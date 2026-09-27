// SPDX-License-Identifier: MIT

use std::fs;
use std::path::PathBuf;

use super::{Claimed, findings, nonblank, split_claims};
use crate::config::Policy;
use crate::diagnostic::{Finding, Severity};

/// Budgets small enough that a handful of fixture lines crosses a hard maximum.
const LIMITS: &str = "\
policy_version = 1

[project]
required_files = []
ignored_directories = []
ignored_path_prefixes = []

[limits]
rust_production_preferred = 3
rust_production_max = 4
rust_test_preferred = 3
rust_test_max = 4
csharp_production_preferred = 3
csharp_production_max = 4
csharp_test_preferred = 3
csharp_test_max = 4
workflow_preferred = 3
workflow_max = 4
markdown_preferred = 3
markdown_max = 4
";

/// Builds a root holding one `a.rs` with `lines` nonblank lines, exempted with `reason`.
fn fixture(name: &str, lines: usize, reason: &str) -> Result<(PathBuf, Policy), String> {
    let root = std::env::temp_dir().join(format!("repo-policy-exc-{name}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("crates/harness/src")).map_err(|error| error.to_string())?;
    let body = (0..lines)
        .map(|index| format!("line {index}\n"))
        .collect::<String>();
    fs::write(root.join("crates/harness/src/a.rs"), body).map_err(|error| error.to_string())?;
    let policy = Policy::parse_text(&format!(
        "{LIMITS}\n[exemptions]\n\"crates/harness/src/a.rs\" = \"{reason}\"\n"
    ))?;
    Ok((root, policy))
}

fn rules(findings: &[Finding]) -> Vec<(&str, Severity)> {
    findings
        .iter()
        .map(|finding| (finding.rule, finding.severity))
        .collect()
}

#[test]
fn separates_the_size_claim_from_the_cited_budget() {
    assert_eq!(
        split_claims("its 322-line wiring remains below the hard limit"),
        Some(Claimed {
            size: Some(322),
            maximum: None
        })
    );
    assert_eq!(
        split_claims("its 632 nonblank lines remain below the 700-line markdown hard limit"),
        Some(Claimed {
            size: Some(632),
            maximum: Some(700)
        })
    );
    assert_eq!(
        split_claims("its 334-line implementation is below the hard limit"),
        Some(Claimed {
            size: Some(334),
            maximum: None
        })
    );
    assert_eq!(
        split_claims("the #559 entry stays beside its neighbours"),
        Some(Claimed {
            size: None,
            maximum: None
        })
    );
}

#[test]
fn two_size_claims_fail_closed() -> Result<(), String> {
    assert_eq!(
        split_claims("its 4-line body and its 9-line sibling belong together"),
        None
    );
    let (root, policy) = fixture(
        "ambiguous",
        4,
        "a module; its 4-line body and its 9-line sibling both belong together",
    )?;
    let found = findings(&root, &policy);
    assert_eq!(rules(&found), [("EXC002", Severity::Error)]);
    assert!(
        found[0].message.contains("more than one line count"),
        "{}",
        found[0].message
    );
    Ok(())
}

#[test]
fn exact_count_passes() -> Result<(), String> {
    let (root, policy) = fixture(
        "exact",
        4,
        "a single bounded module; its 4-line body holds together",
    )?;
    assert!(findings(&root, &policy).is_empty());
    Ok(())
}

#[test]
fn no_count_exemption_stays_valid() -> Result<(), String> {
    let (root, policy) = fixture(
        "nocount",
        4,
        "a single bounded module whose parts belong together",
    )?;
    assert!(findings(&root, &policy).is_empty());
    Ok(())
}

#[test]
fn stale_count_within_the_hard_maximum_is_prose_drift() -> Result<(), String> {
    let (root, policy) = fixture(
        "stale",
        3,
        "a single bounded module; its 9-line body holds together",
    )?;
    let found = findings(&root, &policy);
    assert_eq!(rules(&found), [("EXC002", Severity::Error)]);
    assert!(
        found[0].message.contains("states 9 nonblank lines"),
        "{}",
        found[0].message
    );
    assert!(
        found[0].message.contains("within its hard maximum of 4"),
        "{}",
        found[0].message
    );
    Ok(())
}

#[test]
fn accurate_count_still_over_the_hard_maximum_waives_a_breach() -> Result<(), String> {
    let (root, policy) = fixture(
        "waived",
        9,
        "a bounded module; its 9-line body is over-hard-maximum: #570",
    )?;
    let found = findings(&root, &policy);
    assert!(
        found.is_empty(),
        "an acknowledged breach with an exact count is accepted: {found:?}"
    );
    Ok(())
}

#[test]
fn an_unacknowledged_breach_is_reported_even_with_an_exact_count() -> Result<(), String> {
    let (root, policy) = fixture(
        "hidden",
        9,
        "a single bounded module; its 9-line body holds together",
    )?;
    assert_eq!(
        rules(&findings(&root, &policy)),
        [("EXC003", Severity::Error)]
    );
    Ok(())
}

#[test]
fn a_waiver_marker_needs_a_tracked_issue() -> Result<(), String> {
    let (root, policy) = fixture(
        "badwaiver",
        9,
        "a bounded module; its 9-line body is over-hard-maximum: soon",
    )?;
    let found = findings(&root, &policy);
    assert_eq!(rules(&found), [("EXC003", Severity::Error)]);
    assert!(
        found[0].message.contains("waives a real breach"),
        "{}",
        found[0].message
    );
    Ok(())
}

#[test]
fn a_cited_budget_may_not_contradict_limits() -> Result<(), String> {
    let (root, policy) = fixture(
        "budget",
        2,
        "a module; its 2 nonblank lines stay below the 900-line rust hard limit",
    )?;
    let found = findings(&root, &policy);
    assert_eq!(rules(&found), [("EXC004", Severity::Error)]);
    assert!(
        found[0].message.contains("cites a 900-line hard limit"),
        "{}",
        found[0].message
    );
    assert!(
        found[0].message.contains("limits] sets 4"),
        "{}",
        found[0].message
    );
    Ok(())
}

#[test]
fn a_matching_cited_budget_passes() -> Result<(), String> {
    let (root, policy) = fixture(
        "budget-ok",
        2,
        "a module; its 2 nonblank lines stay below the 4-line rust hard limit",
    )?;
    assert!(findings(&root, &policy).is_empty());
    Ok(())
}

#[test]
fn unparsable_count_prose_is_left_alone() -> Result<(), String> {
    let (root, policy) = fixture(
        "garbage",
        2,
        "a module; its nine-ish line body grew past the budget",
    )?;
    assert!(
        findings(&root, &policy).is_empty(),
        "no numeric claim states no count to verify"
    );
    Ok(())
}

#[test]
fn an_issue_reference_is_not_a_line_claim() {
    assert_eq!(
        split_claims("the #559 entry stays beside its neighbours and its 632-line file is bounded"),
        Some(Claimed {
            size: Some(632),
            maximum: None
        })
    );
}

#[test]
fn a_cited_budget_that_drifted_is_reported() -> Result<(), String> {
    // A copied budget is the failure this guards: a limit is widened in [limits], never in prose.
    let (root, policy) = fixture(
        "drift",
        2,
        "a module; its 2 nonblank lines remain below the 500-line rust hard limit",
    )?;
    let found = findings(&root, &policy);
    assert_eq!(rules(&found), [("EXC004", Severity::Error)]);
    assert!(
        found[0].message.contains("cites a 500-line hard limit"),
        "{}",
        found[0].message
    );
    Ok(())
}

#[test]
fn nonblank_matches_the_size_rule() {
    assert_eq!(nonblank("a\n\n  \nb\n"), 2);
    assert_eq!(nonblank(""), 0);
}

#[test]
fn keeps_the_original_shape_checks() -> Result<(), String> {
    let policy = Policy::parse_text(&format!(
        "{LIMITS}\n[exemptions]\n\
         \"crates/harness/src/gone.rs\" = \"a bounded module that no longer exists here\"\n\
         \"crates/harness/src/a.rs\" = \"short\"\n\
         \"../escape.rs\" = \"a bounded module that tries to escape the repository root\"\n\
         \"/abs.rs\" = \"a bounded module given as an absolute repository path\"\n"
    ))?;
    let root = std::env::temp_dir().join("repo-policy-exc-shape");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    assert_eq!(
        rules(&findings(&root, &policy)),
        [("EXC001", Severity::Error); 4]
    );
    Ok(())
}
