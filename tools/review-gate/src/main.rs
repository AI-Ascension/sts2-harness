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
//! The `gh` subprocess lifecycle -- spawning it, draining its pipes, bounding how
//! long it may run -- lives in [`process`], so this file stays about the gate's own
//! decision and the reporting around it.
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
use std::time::Duration;

mod decision;
mod process;

use decision::{ReviewGateError, Verdict, check_with, require_full_sha, validate_reviews};

/// A git object name: 40 lowercase hex characters.
///
/// GitHub returns the full 40-character SHA for head commits. Short forms are
/// rejected so a truncated or padded field cannot silently compare unequal to the
/// head and mask a real pin, and cannot be mistaken for a real SHA.
pub(crate) const FULL_SHA_LENGTH: usize = 40;

/// How long a single `gh api` call may take before the gate gives up on it.
///
/// The reference implementation passes `timeout=self.timeout` to `subprocess.run`
/// with a default of 60 seconds, and the port dropped it: the Rust seam used
/// `Command::output`, which waits forever. That is a fidelity gap and an
/// availability one. `review-of-record` is a required status check on `main` since
/// ruleset `24104281`, and its job has `timeout-minutes: 5`, so a hung `gh` does not
/// merely waste the job -- it burns the whole five-minute budget and reports a job
/// timeout rather than the actionable "gh api timed out after 60s for <endpoint>".
pub(crate) const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// Thin `gh api` seam for reading pull-request state.
///
/// Shells out to the authenticated `gh` executable, surfaces failures as
/// [`ReviewGateError`], and never swallows an error into an empty-but-successful read.
struct GhRunner {
    gh_path: String,
    timeout: Duration,
}

impl GhRunner {
    fn run(&self, endpoint: &str) -> Result<serde_json::Value, ReviewGateError> {
        // The spawn, the pipe drain and the deadline all live in `process`, so the
        // #707 `ETXTBSY` retry and the #702 timeout stay one reviewed transport seam
        // rather than two constants that can drift apart between call sites.
        let (status, stdout, stderr) = process::run(&self.gh_path, endpoint, self.timeout)?;

        if !status.success() {
            let message = String::from_utf8_lossy(&stderr);
            let message = message.trim();
            let message = if message.is_empty() {
                String::from_utf8_lossy(&stdout).trim().to_owned()
            } else {
                message.to_owned()
            };
            let message = if message.is_empty() {
                String::from("command failed")
            } else {
                message
            };
            return Err(ReviewGateError(format!(
                "gh api failed for {endpoint}: {message}"
            )));
        }
        let raw = String::from_utf8_lossy(&stdout);
        if raw.trim().is_empty() {
            return Err(ReviewGateError(format!(
                "gh api returned an empty body for {endpoint}"
            )));
        }
        serde_json::from_str(&raw)
            .map_err(|_| ReviewGateError(format!("gh api returned a non-JSON body for {endpoint}")))
    }

    fn head_sha(&self, repository: &str, number: u64) -> Result<String, ReviewGateError> {
        let payload = self.run(&format!("repos/{repository}/pulls/{number}"))?;
        let object = payload.as_object().ok_or_else(|| {
            ReviewGateError(format!(
                "pull request response for {repository}#{number} is not an object"
            ))
        })?;
        let sha = object
            .get("head")
            .and_then(|head| head.get("sha"))
            .and_then(serde_json::Value::as_str);
        require_full_sha(sha, &format!("head SHA for {repository}#{number}"))
    }

    fn reviews(&self, repository: &str, number: u64) -> Result<serde_json::Value, ReviewGateError> {
        let payload = self.run(&format!(
            "repos/{repository}/pulls/{number}/reviews?per_page=100"
        ))?;
        // `gh api` may return a list directly, or (with --slurp) a list of pages.
        // Normalize the nested page form before validation.
        if let Some(pages) = payload.as_array()
            && !pages.is_empty()
            && pages.iter().all(serde_json::Value::is_array)
        {
            let mut flattened = Vec::new();
            for page in pages {
                if let Some(entries) = page.as_array() {
                    flattened.extend(entries.iter().cloned());
                }
            }
            return Ok(serde_json::Value::Array(flattened));
        }
        // Validated here, not only in `evaluate`: an error-shaped body such as
        // `{"message": "Not Found"}` arrives on a 200, and reading it as an empty
        // review set would turn a failed read into a decided pass.
        validate_reviews(&payload)?;
        Ok(payload)
    }
}

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
