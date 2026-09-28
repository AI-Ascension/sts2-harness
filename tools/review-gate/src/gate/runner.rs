// SPDX-License-Identifier: MIT

//! The `gh` subprocess seam that reads the API.
//!
//! These drive the real seam so failure is demonstrated, not asserted.

use super::HEAD;
use crate::GhRunner;
use crate::decision::ReviewGateError;
use std::error::Error;
use std::path::PathBuf;

use serde_json::json;

/// A fake `gh` that prints `body` on stdout, `stderr` on stderr, and exits `code`.
fn fake_gh(body: &str, stderr: &str, code: i32) -> Result<PathBuf, Box<dyn Error>> {
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
    })
}

/// `test_nonzero_exit_raises`
#[test]
fn nonzero_exit_raises() -> Result<(), Box<dyn Error>> {
    let path = fake_gh("", "gh: Not Found (HTTP 404)", 1)?;
    let runner = GhRunner {
        gh_path: path.to_string_lossy().into_owned(),
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
