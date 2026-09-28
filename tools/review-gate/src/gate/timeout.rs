// SPDX-License-Identifier: MIT

//! The deadline on the `gh` seam, and the negative control for it.
//!
//! The reference implementation passes `timeout=self.timeout` to `subprocess.run` with a
//! default of 60 seconds. The port dropped it and used `Command::output`, which has no
//! deadline at all, so these tests pin the property the reference has and the port lost.

use super::{HEAD, fake_gh};
use crate::GhRunner;
use crate::decision::ReviewGateError;
use serde_json::json;
use std::error::Error;
use std::path::PathBuf;

/// A fake `gh` that never exits, so the timeout is the only thing that can end it.
///
/// `sleep` rather than a busy loop: the point is to model a `gh` blocked on the
/// network, and a spinning child would make this test a CPU-burner on a host that
/// is already loaded.
fn hanging_gh() -> Result<PathBuf, Box<dyn Error>> {
    static NEXT_HANGING_GH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let nonce = NEXT_HANGING_GH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "sts2-review-gate-hang-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("gh");
    std::fs::write(&path, "#!/bin/sh\nsleep 3600\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// A hung `gh` is reported as a timeout, not waited on forever.
///
/// The port used `Command::output`, which has no deadline, so this call returned
/// only when the child did. This is the regression test for that: the stub never
/// exits, so the assertion can only pass if something enforces the timeout. The
/// production timeout is 60s, which is far too long to spend in a test, so this
/// drives a runner configured with a short one.
#[test]
fn a_hung_gh_is_reported_as_a_timeout() -> Result<(), Box<dyn Error>> {
    let runner = GhRunner {
        gh_path: hanging_gh()?.to_string_lossy().into_owned(),
        timeout: std::time::Duration::from_millis(200),
    };
    let started = std::time::Instant::now();
    let outcome = runner.head_sha("AI-Ascension/sts2-harness", 693);
    let elapsed = started.elapsed();
    let Err(error) = outcome else {
        return Err("a hung `gh` must not yield a verdict".into());
    };
    assert!(
        matches!(&error, ReviewGateError(message) if message.contains("timed out")),
        "a hung `gh` reported {error:?} rather than a timeout, so the deadline is not enforced"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the call took {elapsed:?}, so it waited on the child rather than on the deadline"
    );
    Ok(())
}

/// The timeout names the endpoint, so the check-run says which call hung.
#[test]
fn the_timeout_names_the_endpoint_it_gave_up_on() -> Result<(), Box<dyn Error>> {
    let runner = GhRunner {
        gh_path: hanging_gh()?.to_string_lossy().into_owned(),
        timeout: std::time::Duration::from_millis(200),
    };
    let Err(error) = runner.reviews("AI-Ascension/sts2-harness", 693) else {
        return Err("a hung `gh` must not yield reviews".into());
    };
    assert!(
        matches!(&error, ReviewGateError(message)
            if message.contains("timed out") && message.contains("pulls/693/reviews")),
        "the timeout message {error:?} does not name the endpoint, so a failing check-run would \
         not say which call hung"
    );
    Ok(())
}

/// A `gh` that finishes inside the deadline is unaffected by having one.
///
/// The negative control for the two tests above. Without it, a runner that failed
/// every call immediately would satisfy them.
#[test]
fn a_gh_that_finishes_in_time_still_succeeds() -> Result<(), Box<dyn Error>> {
    let runner = GhRunner {
        gh_path: fake_gh(&json!({"head": {"sha": HEAD}}).to_string(), "", 0)?
            .to_string_lossy()
            .into_owned(),
        timeout: std::time::Duration::from_secs(30),
    };
    assert_eq!(runner.head_sha("AI-Ascension/sts2-harness", 693)?, HEAD);
    Ok(())
}
