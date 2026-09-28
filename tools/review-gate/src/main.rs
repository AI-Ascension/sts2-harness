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
use std::io::Read;
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

mod decision;

use decision::{ReviewGateError, Verdict, check_with, require_full_sha, validate_reviews};

/// A git object name: 40 lowercase hex characters.
///
/// GitHub returns the full 40-character SHA for head commits. Short forms are
/// rejected so a truncated or padded field cannot silently compare unequal to the
/// head and mask a real pin, and cannot be mistaken for a real SHA.
pub(crate) const FULL_SHA_LENGTH: usize = 40;

/// `ETXTBSY`, the errno `execve` returns when the image is still open for writing anywhere.
pub(crate) const TEXT_FILE_BUSY: i32 = 26;

/// How long to keep re-trying before giving up and surfacing the original error.
const DEADLINE: Duration = Duration::from_secs(10);

/// How long to wait between attempts. The holder of the descriptor closes it within
/// microseconds, so a long backoff would only add latency; short is bounded by the deadline.
const BACKOFF: Duration = Duration::from_millis(2);

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

/// How often the child is polled while waiting for it.
///
/// Short enough that a timeout is not reported materially later than it happened,
/// long enough that the wait is not a busy loop.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(25);

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
        let mut command = Command::new(&self.gh_path);
        command.arg("api").arg(endpoint);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Spawned through the `ETXTBSY` retry rather than a bare `spawn()`. The timeout work
        // below replaces `Command::output()` with an explicit spawn so the child can be polled,
        // and a bare spawn is exactly what #707 was about: a `fork`ed sibling can inherit another
        // thread's write descriptor and the kernel refuses the exec. The retry is orthogonal to
        // the timeout, so dropping it here would silently revert #711 the moment this lands.
        let mut child = spawn_retrying_text_busy(&mut command)
            .map_err(|error| ReviewGateError(format!("unable to run {}: {error}", self.gh_path)))?;

        // Both pipes are drained on their own threads. Reading them inline after
        // the child exits would deadlock the other way round: a child that fills a
        // pipe buffer blocks in `write` while we block in `read`, and neither side
        // makes progress. Draining concurrently lets the child always finish
        // writing, however large the body is.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_reader = thread::spawn(move || read_pipe(stdout));
        let stderr_reader = thread::spawn(move || read_pipe(stderr));

        let status = self.wait_with_timeout(&mut child, endpoint)?;
        let stdout = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default();

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

    /// Wait for `child`, killing it if it outlives the timeout.
    ///
    /// The kill is followed by a `wait` on purpose. A killed child is not reaped
    /// until someone waits on it, so skipping that would trade a hung subprocess for
    /// an unreaped one, and the reader threads spawned in [`run`] would be the only
    /// things still holding its pipes.
    ///
    /// [`run`]: GhRunner::run
    fn wait_with_timeout(
        &self,
        child: &mut Child,
        endpoint: &str,
    ) -> Result<ExitStatus, ReviewGateError> {
        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                // Still running. Fall through to the deadline check.
                Ok(None) => {}
                Err(error) => {
                    reap(child);
                    return Err(ReviewGateError(format!(
                        "unable to wait for {}: {error}",
                        self.gh_path
                    )));
                }
            }
            if Instant::now() >= deadline {
                reap(child);
                return Err(ReviewGateError(format!(
                    "gh api timed out after {}s for {endpoint}",
                    self.timeout.as_secs()
                )));
            }
            thread::sleep(WAIT_POLL_INTERVAL);
        }
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

/// Spawn `command`, retrying only while it is refused with `ETXTBSY`.
///
/// # Why this is needed
///
/// `execve` refuses to run a file that any live descriptor holds open for writing, and returns
/// `ETXTBSY` (`errno 26`) when it does. The stubs this crate's own tests execute are produced by
/// [`fs::write`](std::fs::write) followed by `chmod 0o755` and then exec, and the test binary runs
/// its tests in parallel threads. So while thread A is between its own `write` and its own `exec`,
/// thread B can `fork`, inherit that still-open write descriptor, and `exec` -- and whichever
/// `exec` lands first loses.
///
/// This is a genuine kernel race, not a defect in the stubs. Each stub gets its own directory and
/// filename, so the two threads never touch the same path; the coupling is the inherited
/// descriptor, not the name. It is rare enough to be unreproducible on demand: #707 records one
/// failure in 40 full-suite runs, and 200 consecutive clean full-suite runs on this host did not
/// reproduce it at all. That rarity is why it survived a fix that removed a genuine path collision
/// and then declared victory on 12 clean runs -- a rate that cannot distinguish "fixed" from
/// "almost never hit", which is why the regression test here manufactures the condition instead of
/// looping the suite and hoping. Because `review-of-record` is a required check on every pull
/// request, an occurrence blocks unrelated work at random, after the author has already done the
/// right thing, which trains re-running instead of reading. See #707.
///
/// This is the same remedy the harness test support already uses, and for the same measured
/// reason: `crates/harness/tests/support/runtime_v4_executable_composition_process/spawn.rs`
/// measured **5 failures in 240 spawns** without the retry and **0 in 240** with it.
///
/// Retrying is sound because the descriptor that caused the refusal is always closed by its
/// owner -- `fs::write` returns only after the `File` drops -- so the condition is transient by
/// construction and the deadline cannot be outlived by a *persistent* one. A script held open for
/// writing by some other process would instead burn the full [`DEADLINE`] and then report the
/// original `ETXTBSY` rather than hanging forever, and any other errno propagates on the first
/// attempt with no delay at all.
fn spawn_retrying_text_busy(command: &mut Command) -> Result<std::process::Child, std::io::Error> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.raw_os_error() == Some(TEXT_FILE_BUSY) && Instant::now() < deadline =>
            {
                thread::sleep(BACKOFF);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Drain one pipe to a `Vec<u8>` on a dedicated thread.
///
/// A read error yields whatever was collected before it, which is the same
/// fail-closed direction as every other error here: a truncated body cannot parse
/// as the JSON the caller requires, so a partial read becomes a reported failure
/// rather than a silently short one.
fn read_pipe<R: Read + Send + 'static>(pipe: Option<R>) -> Vec<u8> {
    let Some(mut pipe) = pipe else {
        return Vec::new();
    };
    let mut collected = Vec::new();
    let _ = pipe.read_to_end(&mut collected);
    collected
}

/// Kill a child and reap it, so a timeout does not leave a process behind.
fn reap(child: &mut Child) {
    // Both calls are best-effort by design. A child that already exited between the
    // deadline check and the kill reports failure here; the `wait` that follows is
    // still correct, because it is the call that reaps.
    let _ = child.kill();
    let _ = child.wait();
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
