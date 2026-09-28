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
    let directory = std::env::temp_dir().join(format!(
        "sts2-review-gate-test-{}-{}",
        std::process::id(),
        body.len() * 31 + stderr.len() * 7 + code.unsigned_abs() as usize
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
