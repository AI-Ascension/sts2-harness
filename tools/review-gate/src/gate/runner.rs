// SPDX-License-Identifier: MIT

//! The `gh` subprocess seam that reads the API.
//!
//! These drive the real seam so failure is demonstrated, not asserted.

use super::HEAD;
use crate::decision::ReviewGateError;
use crate::{GH_TIMEOUT, GhRunner};
use std::error::Error;
use std::fs::File;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

/// A fake `gh` that prints `body` on stdout, `stderr` on stderr, and exits `code`.
pub(super) fn fake_gh(body: &str, stderr: &str, code: i32) -> Result<PathBuf, Box<dyn Error>> {
    // The directory name must be unique per CALL, not per body. Two tests here legitimately pass
    // the same body -- `real_subprocess_roundtrip_extracts_head_and_reviews` and
    // `head_sha_extracted_from_payload` both use `{"head": {"sha": HEAD}}` -- and a name derived
    // from the body's length gave them the same path. They then raced: one `execve`d the stub
    // while the other was still writing it, and the loser failed with `ETXTBSY` ("Text file busy",
    // os error 26). That is a real defect rather than a flake, because the test outcome depended
    // on scheduling.
    //
    // The counter is process-local and `fetch_add` is atomic, so every call in this process gets a
    // distinct name, and the process id separates concurrent test binaries. `create_dir_all`
    // tolerates the collision rather than being the thing that reports it, which is why a lost race
    // used to surface much later as a confusing exec failure instead of here.
    static NEXT_FAKE_GH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let nonce = NEXT_FAKE_GH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "sts2-review-gate-test-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("gh");
    let script = format!(
        "#!/bin/sh\ncat <<'BODY'\n{body}\nBODY\ncat >&2 <<'ERR'\n{stderr}\nERR\nexit {code}\n"
    );
    std::fs::write(&path, script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// A runner wired to a fake `gh` that answers every call with `body`.
fn runner_for(body: &str) -> Result<GhRunner, Box<dyn Error>> {
    let path = fake_gh(body, "", 0)?;
    Ok(GhRunner {
        gh_path: path.to_string_lossy().into_owned(),
        timeout: GH_TIMEOUT,
    })
}

/// `test_nonzero_exit_raises`
#[test]
fn nonzero_exit_raises() -> Result<(), Box<dyn Error>> {
    let path = fake_gh("", "gh: Not Found (HTTP 404)", 1)?;
    let runner = GhRunner {
        gh_path: path.to_string_lossy().into_owned(),
        timeout: GH_TIMEOUT,
    };
    let outcome = runner.head_sha("AI-Ascension/.github", 49);
    assert!(matches!(outcome, Err(ReviewGateError(_))));
    Ok(())
}

/// `test_empty_body_raises`
#[test]
fn empty_body_raises() -> Result<(), Box<dyn Error>> {
    let runner = runner_for("")?;
    assert!(runner.head_sha("AI-Ascension/.github", 1).is_err());
    Ok(())
}

/// `test_non_json_body_raises`
#[test]
fn non_json_body_raises() -> Result<(), Box<dyn Error>> {
    let runner = runner_for("<html>nope</html>")?;
    assert!(runner.head_sha("AI-Ascension/.github", 1).is_err());
    Ok(())
}

/// `test_error_object_body_is_rejected_not_accepted`
///
/// A 200 carrying an error-shaped body is a failed read, not an empty result. Read
/// as an empty review set it would turn a failed read into a decided pass.
#[test]
fn error_object_body_is_rejected_not_accepted() -> Result<(), Box<dyn Error>> {
    let runner = runner_for(&json!({"message": "Not Found"}).to_string())?;
    assert!(runner.reviews("AI-Ascension/.github", 1).is_err());
    Ok(())
}

/// `test_head_payload_that_is_not_object_raises`
#[test]
fn head_payload_that_is_not_object_raises() -> Result<(), Box<dyn Error>> {
    let runner = runner_for(&json!([{"sha": HEAD}]).to_string())?;
    assert!(runner.head_sha("AI-Ascension/.github", 1).is_err());
    Ok(())
}

/// `test_missing_executable_raises`
#[test]
fn missing_executable_raises() -> Result<(), Box<dyn Error>> {
    let runner = GhRunner {
        gh_path: String::from("/nonexistent/gh-binary"),
        timeout: GH_TIMEOUT,
    };
    let outcome = runner.head_sha("AI-Ascension/.github", 49);
    assert!(matches!(outcome, Err(ReviewGateError(_))));
    Ok(())
}

/// `test_real_subprocess_roundtrip_extracts_head_and_reviews`
///
/// End-to-end through the actual subprocess seam.
#[test]
fn real_subprocess_roundtrip_extracts_head_and_reviews() -> Result<(), Box<dyn Error>> {
    let runner = runner_for(&json!({"head": {"sha": HEAD}}).to_string())?;
    assert_eq!(runner.head_sha("AI-Ascension/.github", 1)?, HEAD);
    Ok(())
}

/// `test_two_runners_with_one_payload_get_distinct_paths`
///
/// The property the per-call nonce exists to provide, asserted rather than assumed: the two tests
/// above deliberately share a body, so a name derived from the body alone hands both the same
/// path and they race on the executable. Distinct paths are the fix; both runners must still work,
/// so a fix that made the paths differ by breaking the call would not pass either.
#[test]
fn two_runners_with_one_payload_get_distinct_paths() -> Result<(), Box<dyn Error>> {
    let body = json!({"head": {"sha": HEAD}}).to_string();
    let first = runner_for(&body)?;
    let second = runner_for(&body)?;
    assert_ne!(
        first.gh_path, second.gh_path,
        "two fake `gh` builds from one payload must not share a path"
    );
    assert_eq!(first.head_sha("AI-Ascension/.github", 1)?, HEAD);
    assert_eq!(second.head_sha("AI-Ascension/sts2-harness", 693)?, HEAD);
    Ok(())
}

/// `test_head_sha_extracted_from_payload`
#[test]
fn head_sha_extracted_from_payload() -> Result<(), Box<dyn Error>> {
    let runner = runner_for(&json!({"head": {"sha": HEAD}}).to_string())?;
    assert_eq!(runner.head_sha("AI-Ascension/sts2-harness", 693)?, HEAD);
    Ok(())
}

/// `test_head_sha_missing_in_payload_raises`
#[test]
fn head_sha_missing_in_payload_raises() -> Result<(), Box<dyn Error>> {
    let runner = runner_for(&json!({"head": {}}).to_string())?;
    assert!(runner.head_sha("AI-Ascension/.github", 1).is_err());
    Ok(())
}

/// `test_reviews_nested_pages_flattened`
///
/// The paginated field is a list of pages, each a list of reviews.
#[test]
fn reviews_nested_pages_flattened() -> Result<(), Box<dyn Error>> {
    // `gh api` may return a list directly, or (with `--slurp`) a list of pages.
    let body = json!([[{"id": 1}], [{"id": 2}]]);
    let runner = runner_for(&body.to_string())?;
    let reviews = runner.reviews("AI-Ascension/.github", 1)?;
    assert_eq!(reviews.as_array().map(Vec::len), Some(2));
    Ok(())
}

/// `test_reviews_not_list_raises`
#[test]
fn reviews_not_list_raises() -> Result<(), Box<dyn Error>> {
    let runner = runner_for(&json!({"message": "Not Found"}).to_string())?;
    let outcome = runner.reviews("AI-Ascension/.github", 1);
    assert!(matches!(outcome, Err(ReviewGateError(_))));
    Ok(())
}

/// A regression that fails on the pre-#707 implementation.
///
/// The real flake is a `fork`-inherits-a-write-descriptor race, rare enough that asserting "the
/// suite passes" cannot detect it -- #707 reports one failure in 40 full-suite runs, and 200
/// consecutive clean full-suite runs on the same host did not reproduce it. That is exactly what
/// let it survive a fix verified on 12 clean runs. So this test does not run the suite in a loop and
/// hope. It *manufactures* the condition deterministically: it holds a write descriptor open on
/// the very stub the runner is about to exec, in the same way an overlapping `fork` would, releases
/// it the way the forked child would, and asserts the runner still succeeds.
///
/// Against the old `Command::output()` this fails with `Text file busy`. It passes only because
/// `spawn_retrying_text_busy` waits for the holder to close, which is the actual defect being
/// fixed.
///
/// The holder is released on a background thread rather than inline, and that is load-bearing
/// rather than incidental. The retry is sound only because the descriptor is *transiently* held
/// and always closed by its owner; a holder that never released would simply burn the full
/// deadline and still fail. Releasing it from another thread reproduces the real ordering --
/// holder's window overlapping the exec, then holder's owner closing -- so the test exercises the
/// wait rather than the timeout.
#[test]
fn exec_succeeds_while_another_descriptor_holds_the_stub_for_writing() -> Result<(), Box<dyn Error>>
{
    let runner = runner_for(&json!({"head": {"sha": HEAD}}).to_string())?;

    // Re-open the runner's own stub for writing and hold it across the exec, which is the state
    // the kernel refuses. `fs::write` inside `fake_gh` is long since closed by now, so this
    // descriptor is the only one standing in for the forked child. Opened write-only and *not*
    // truncated or written: a write would corrupt the shebang and make the stub's output empty,
    // which is a different failure than the one under test.
    let holder = File::options()
        .write(true)
        .truncate(false)
        .open(&runner.gh_path)?;

    // Stand in for the forked child: the descriptor is open when the exec is attempted, and its
    // owner closes it a moment later, unprompted.
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        drop(holder);
    });

    let outcome = runner.head_sha("AI-Ascension/.github", 1);
    // Propagated rather than unwrapped: the crate denies `expect`, and a panicking releaser
    // would leave the descriptor open, so the failure must be reported, not swallowed.
    releaser
        .join()
        .map_err(|_| "the releasing thread panicked")?;
    assert_eq!(
        outcome?, HEAD,
        "the runner must survive a concurrent write handle"
    );
    Ok(())
}

/// The control for the regression above, so it cannot pass vacuously.
///
/// If the retry were removed, the test above would still pass for the wrong reason if the
/// descriptor it opened were somehow not on the exec'd path. This asserts the refusal is real
/// and reachable at all: a plain, un-retried `Command` on the same held-open stub must fail with
/// `ETXTBSY`. Without this, a broken regression that never provoked the condition would look
/// identical to a working fix.
#[test]
fn an_unretried_exec_of_a_held_open_stub_is_refused() -> Result<(), Box<dyn Error>> {
    let runner = runner_for(&json!({"head": {"sha": HEAD}}).to_string())?;
    let holder = File::options()
        .write(true)
        .truncate(false)
        .open(&runner.gh_path)?;
    // Never written: a write would truncate the shebang and turn the control into a different
    // failure. The descriptor being open for writing is the whole condition.

    let unretried = std::process::Command::new(&runner.gh_path)
        .arg("api")
        .arg("repos/AI-Ascension/.github/pulls/1")
        .output();
    drop(holder);

    let Err(error) = unretried else {
        return Err("an un-retried exec of a held-open stub must fail".into());
    };
    assert_eq!(
        error.raw_os_error(),
        Some(crate::gh_api::TEXT_FILE_BUSY),
        "the control must fail with ETXTBSY, not some unrelated error: {error}"
    );
    Ok(())
}
