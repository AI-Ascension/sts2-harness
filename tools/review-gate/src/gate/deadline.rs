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
    let (_guard, stub, pid_file) = grandchild_gh()?;
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

    // `kill -0` alone cannot answer this, which is #722. It sends no signal and only runs the
    // existence and permission checks, so it succeeds for a **zombie** as well as for a running
    // process: the pid still resolves until the parent reaps. A zombie is not running, is not
    // holding the stdout pipe, and is precisely what the group kill is supposed to leave behind,
    // so counting one as "alive" made this test fail against the fixed code -- measured on merged
    // `main` at 5 failures in 20 runs, each one sampling inside the reaping window.
    let alive = descendant_is_running(pid);
    assert!(
        !alive,
        "pid {pid} survived the timeout, so only the direct child was killed and the \
         descendant `gh` started is still running"
    );
    Ok(())
}

/// How many probes `descendant_is_running` makes before calling a pid still running.
///
/// A descendant of a killed child is re-parented when its parent dies, and it is not reapable
/// until that completes, so "present in the process table" lags "dead" by a scheduling-dependent
/// interval. The loop exits on the first probe that sees it gone, so this bound only costs time
/// when the descendant really is still running -- which is the case this test must catch.
const REAP_SETTLE_PROBES: usize = 200;

/// The gap between probes, chosen so the full bound is about a second.
const REAP_SETTLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);

/// Whether `pid` is still *running*, as opposed to merely present in the process table.
///
/// `kill -0` cannot make that distinction, so this asks the kernel twice: existence via
/// `kill -0`, and then, where the platform exposes it, the one-character state from `/proc`. A
/// `Z` (zombie) or `X` (dead) state means the process is gone for every purpose this test cares
/// about. Where `/proc` is absent the answer degrades to the `kill -0` behaviour plus the settle
/// window, which is no weaker than the check this replaces.
fn descendant_is_running(pid: i32) -> bool {
    let procfs = std::path::Path::new("/proc/self/stat").is_file();
    for _ in 0..REAP_SETTLE_PROBES {
        if !pid_exists(pid) {
            return false;
        }
        if procfs && matches!(process_state(pid), Some('Z' | 'X')) {
            return false;
        }
        std::thread::sleep(REAP_SETTLE_INTERVAL);
    }
    true
}

/// Whether `pid` still resolves, via `kill -0`, which needs no privilege here because the group
/// kill ran as this same user.
fn pid_exists(pid: i32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The one-character process state from `/proc/<pid>/stat`, or `None` if it cannot be read.
///
/// The second field is the executable name in parentheses and may itself contain spaces and
/// parentheses, so the state is taken from the token after the **last** `)` rather than by
/// counting fields from the start.
fn process_state(pid: i32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = &stat[stat.rfind(')')? + 1..];
    after_name.split_whitespace().next()?.chars().next()
}

/// A stub `gh` that backgrounds a long `sleep` and then blocks forever.
///
/// Returns the stub path and the file it writes the background pid into. The background job
/// is what makes this a grandchild case: the recorded pid belongs to a child of the script,
/// not to the pid the gate spawned, so a kill aimed at the spawned pid alone misses it.
///
/// The guard comes back with the paths because the stub has to still be on disk when the
/// runner `exec`s it, which is after this returns, and the test reads the pid file while the
/// guard is alive.
///
/// #719: this helper was added by #718 after #713 was closed, and it reintroduced the exact
/// leak #713 fixed -- a `create_dir_all` per call that nothing ever removed, so every run of
/// the suite left an `sts2-review-gate-grandchild-*` directory in `$TMPDIR` behind. `TempDir`
/// is the landed remedy for this shape, and `fake_gh_leaves_no_directory_behind` is the
/// control that keeps it honest.
#[cfg(unix)]
pub(super) fn grandchild_gh() -> Result<(TempDir, PathBuf, PathBuf), Box<dyn Error>> {
    let directory = TempDir::new("grandchild")?;
    let path = directory.path.join("gh");
    let pid_file = directory.path.join("background.pid");
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
    Ok((directory, path, pid_file))
}
