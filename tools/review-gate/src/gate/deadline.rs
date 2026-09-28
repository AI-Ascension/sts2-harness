// SPDX-License-Identifier: MIT

//! The deadline the `gh` seam enforces, and the control that keeps it honest.
//!
//! #702: the port dropped the reference's 60s `subprocess.run(timeout=...)`, so a
//! hung `gh` burned the whole `review-of-record` job budget instead of reporting
//! an actionable timeout. These drive a runner with a short deadline, because the
//! production 60s is far too long to spend inside a test.

use super::HEAD;
use super::runner::fake_gh;
use super::tempdir::TempDir;
use crate::decision::ReviewGateError;
use crate::gh_api::GhRunner;
use std::error::Error;
use std::path::PathBuf;

use serde_json::json;

/// A fake `gh` that never exits, so the timeout is the only thing that can end it.
///
/// `sleep` rather than a busy loop: the point is to model a `gh` blocked on the
/// network, and a spinning child would make this test a CPU-burner on a host that
/// is already loaded.
///
/// The guard comes back with the path because the stub has to still be on disk when the
/// runner `exec`s it, which is after this returns. #713: this helper had the same leak as
/// `fake_gh` -- a directory created per call and never removed.
fn hanging_gh() -> Result<(TempDir, PathBuf), Box<dyn Error>> {
    let directory = TempDir::new("hang")?;
    let path = directory.path.join("gh");
    std::fs::write(&path, "#!/bin/sh\nsleep 3600\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok((directory, path))
}

/// A runner bound to a `gh` that will never return, and a deadline short enough to test.
fn hanging_runner() -> Result<(TempDir, GhRunner), Box<dyn Error>> {
    let (guard, path) = hanging_gh()?;
    let runner = GhRunner {
        gh_path: path.to_string_lossy().into_owned(),
        timeout: std::time::Duration::from_millis(200),
    };
    Ok((guard, runner))
}

/// A hung `gh` is reported as a timeout, not waited on forever.
///
/// The port used `Command::output`, which has no deadline, so this call returned
/// only when the child did. This is the regression test for that: the stub never
/// exits, so the assertion can only pass if something enforces the timeout.
#[test]
fn a_hung_gh_is_reported_as_a_timeout() -> Result<(), Box<dyn Error>> {
    let (_guard, runner) = hanging_runner()?;
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
    let (_guard, runner) = hanging_runner()?;
    let outcome = runner.reviews("AI-Ascension/sts2-harness", 693);
    let Err(error) = outcome else {
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
    let (_guard, path) = fake_gh(&json!({"head": {"sha": HEAD}}).to_string(), "", 0)?;
    let runner = GhRunner {
        gh_path: path.to_string_lossy().into_owned(),
        timeout: std::time::Duration::from_secs(30),
    };
    assert_eq!(runner.head_sha("AI-Ascension/sts2-harness", 693)?, HEAD);
    Ok(())
}

/// A timeout kills the whole process group, not just the `gh` it spawned directly.
///
/// # Why this is a separate test from the two above
///
/// #702 requires "correct child reaping", and says why killing the direct child is not
/// enough: `gh` "may have spawned its own children, and a timeout path that leaves a
/// zombie or an unreaped process is worse than the current behaviour". The two tests above
/// cannot see that difference -- they hang the stub's own shell, and a kill of the direct
/// child happens to end them either way.
///
/// The stub here is deliberately a *grandparent* process: the script backgrounds a long
/// `sleep` and records its pid, then blocks. Killing only the script leaves the `sleep`
/// alive and holding the stdout pipe, so the drain threads never see EOF. The test then
/// asserts the recorded pid is gone, which is the assertion that distinguishes a group
/// kill from a child kill.
#[cfg(unix)]
#[test]
fn a_timeout_kills_the_whole_process_group() -> Result<(), Box<dyn Error>> {
    let (stub, pid_file) = grandchild_gh()?;
    let runner = GhRunner {
        gh_path: stub.to_string_lossy().into_owned(),
        timeout: std::time::Duration::from_millis(200),
    };
    let Err(error) = runner.head_sha("AI-Ascension/sts2-harness", 693) else {
        return Err("a hung `gh` yielded a verdict instead of a timeout".into());
    };
    assert!(
        matches!(&error, ReviewGateError(message) if message.contains("timed out")),
        "expected a timeout, got {error:?}"
    );

    // The stub writes the pid file before it blocks, so it exists by the time the deadline
    // expires. Reading it rather than assuming a layout means a failure below names the pid
    // that survived, rather than reporting only that "something" survived.
    let recorded = std::fs::read_to_string(&pid_file)
        .map_err(|error| format!("the stub wrote no pid file at {pid_file:?}: {error}"))?;
    let pid: i32 = recorded
        .trim()
        .parse()
        .map_err(|error| format!("the stub recorded {recorded:?}, which is not a pid: {error}"))?;

    // `kill -0` is the portable liveness probe: it succeeds for a live process and fails with
    // ESRCH for a reaped one, and it needs no privilege because the group kill ran as this
    // same user.
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    assert!(
        !alive,
        "pid {pid} survived the timeout, so only the direct child was killed and the \
         descendant `gh` started is still running"
    );
    Ok(())
}

/// A stub `gh` that backgrounds a long `sleep` and then blocks forever.
///
/// Returns the stub path and the file it writes the background pid into. The background job
/// is what makes this a grandchild case: the recorded pid belongs to a child of the script,
/// not to the pid the gate spawned, so a kill aimed at the spawned pid alone misses it.
#[cfg(unix)]
fn grandchild_gh() -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    static NEXT_GRANDCHILD_GH: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    let nonce = NEXT_GRANDCHILD_GH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "sts2-review-gate-grandchild-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("gh");
    let pid_file = directory.join("background.pid");
    // `sleep 3600 &` is the descendant: it outlives the script and inherits stdout, so it
    // keeps the pipe open after the script itself is killed. `wait` then blocks the script
    // forever, which is the hang the deadline has to end.
    let script = format!(
        "#!/bin/sh\nsleep 3600 &\necho $! > {}\nwait\n",
        pid_file.display()
    );
    std::fs::write(&path, script)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok((path, pid_file))
}
