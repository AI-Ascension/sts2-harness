// SPDX-License-Identifier: MIT

//! The `gh api` seam: a bounded, drain-safe, transient-tolerant subprocess read.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::decision::{ReviewGateError, require_full_sha, validate_reviews};
use crate::reap::reap;

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
/// timeout rather than the actionable "gh api timed out after 60s for `<endpoint>`".
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
pub(crate) struct GhRunner {
    pub(crate) gh_path: String,
    pub(crate) timeout: Duration,
}

impl GhRunner {
    fn run(&self, endpoint: &str) -> Result<serde_json::Value, ReviewGateError> {
        let mut command = Command::new(&self.gh_path);
        command.arg("api").arg(endpoint);
        // The same stdio wiring `Command::output()` applies, which a bare `spawn` would not:
        // stdout and stderr are captured for the caller's own error reporting, and stdin is
        // closed rather than inherited so `gh` can never block waiting on the gate's terminal.
        // Without this the retry would be faithful and the tool would still be broken, because
        // the captured body would be empty and every read would fail closed.
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The child leads its own process group, so a timeout can terminate everything
        // `gh` started rather than only `gh` itself. #702 calls this out explicitly:
        // killing the direct child "is not sufficient; it may have spawned its own
        // children, and a timeout path that leaves a zombie or an unreaped process is
        // worse than the current behaviour". See [`reap`] for what the kill covers,
        // and for the platform where it covers less.
        //
        // `process_group` comes from `CommandExt`, so it needs the unix-qualified
        // import; there is no equivalent on the Windows path.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        // #707: retry while the exec is refused with ETXTBSY, because a stub or image that some
        // other thread still holds open for writing is transient by construction.
        // #702: once the child exists, bound the wait, because `gh` blocked on the network is not.
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

    pub(crate) fn head_sha(
        &self,
        repository: &str,
        number: u64,
    ) -> Result<String, ReviewGateError> {
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

    pub(crate) fn reviews(
        &self,
        repository: &str,
        number: u64,
    ) -> Result<serde_json::Value, ReviewGateError> {
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
