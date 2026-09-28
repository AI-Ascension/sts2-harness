// SPDX-License-Identifier: MIT

//! Fail-closed gate: require a review of record pinned to the pull-request head.
//!
//! `CONTRIBUTING.md` says a green run does not substitute for review, but before this
//! gate nothing in this repository enforced it. This is that check, in Rust.
//!
//! It is a port of `AI-Ascension/.github`'s `tools/review_gate.py`, which is the
//! reviewed reference implementation and the design `.github#49` called for. The
//! port exists because this repository's `LANG001` rule prohibits Python source
//! and `repo-policy --strict` enforces it, so the reference could not be vendored
//! verbatim. The decision logic below is deliberately the same, and
//! `tools/review-gate/src/tests.rs` pins every branch the reference's own tests pin.
//!
//! ## Design constraints, all fail-closed
//!
//! - A review counts only when `commit_id` equals the pull request's *head* SHA, not
//!   merely that some review exists. A review of an older commit does not review the
//!   code that landed.
//! - Reviews of *any* state count. This org's shared merging account receives HTTP
//!   422 on `APPROVE`, so every review of record is a `COMMENT` review. Requiring
//!   `APPROVED` would block every merge. Only the pin is load-bearing.
//! - Every review on the pull request is considered, not just the most recent one.
//! - Anything the tool cannot determine is a *failure*, never a pass: an API error, a
//!   malformed response, a missing or unparseable head SHA, an empty selection, or a
//!   garbage body. An instrument that selects zero items and exits 0 is
//!   indistinguishable from a real pass, so the exit status and the decision are
//!   derived from the same reviewed data.

use std::env;
use std::process::ExitCode;

mod decision;
mod gh_api;

use decision::{ReviewGateError, Verdict, check_with};
use gh_api::{GH_TIMEOUT, GhRunner};

/// A git object name: 40 lowercase hex characters.
///
/// GitHub returns the full 40-character SHA for head commits. Short forms are
/// rejected so a truncated or padded field cannot silently compare unequal to the
/// head and mask a real pin, and cannot be mistaken for a real SHA.
pub(crate) const FULL_SHA_LENGTH: usize = 40;

/// Fetch the pull request's head and reviews, then evaluate them.
///
/// Any undetermined state returns an error instead of a verdict, so a caller cannot
/// mistake a failed read for a passing gate.
fn check(repository: &str, number: u64) -> Result<Verdict, ReviewGateError> {
    let runner = GhRunner {
        gh_path: String::from("gh"),
        timeout: GH_TIMEOUT,
    };
    let head = runner.head_sha(repository, number)?;
    let reviews = runner.reviews(repository, number)?;
    check_with(&reviews, &head)
}

fn main() -> ExitCode {
    let mut repository = None;
    let mut number = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--repository" => repository = arguments.next(),
            "--number" => number = arguments.next(),
            other => {
                eprintln!("unexpected argument: {other}");
                return ExitCode::from(2);
            }
        }
    }
    let (Some(repository), Some(number)) = (repository, number) else {
        eprintln!("--repository owner/name and --number are both required");
        return ExitCode::from(2);
    };
    // Parsed as an integer, so an unresolved value is a hard error rather than a
    // silent zero.
    let Ok(number) = number.parse::<u64>() else {
        eprintln!("--number must be an integer, not {number:?}");
        return ExitCode::from(2);
    };

    let report = match check(&repository, number) {
        Ok(verdict) => serde_json::json!({
            "ok": verdict.ok,
            "reason": verdict.reason,
            "head_sha": verdict.head_sha,
            "review_of_record_id": verdict.review_of_record_id,
        }),
        // Fail closed and loud: an undetermined review state is a failure, and the
        // reason is printed so the check-run is actionable.
        Err(error) => serde_json::json!({
            "ok": false,
            "reason": format!("review state undetermined: {error}"),
            "head_sha": serde_json::Value::Null,
            "review_of_record_id": serde_json::Value::Null,
        }),
    };
    let ok = report["ok"].as_bool().unwrap_or(false);
    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| String::from("{\"ok\":false}"))
    );
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
#[path = "gate/mod.rs"]
mod tests;
