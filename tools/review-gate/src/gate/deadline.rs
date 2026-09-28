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

/// How long a killed descendant is given to leave the process table.
///
/// The group kill is `SIGKILL`, so the descendant is gone from the scheduler essentially at once
/// and this only has to cover the bookkeeping after it. It is generous because exceeding it is a
/// real failure worth reporting, not a nuisance to be tuned away: a descendant still running
/// seconds after its group's `SIGKILL` means the signal did not reach it, which is the defect
/// this test exists to catch.
const DESCENDANT_EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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

    // The stub writes the pid file before it blocks, but the deadline can beat the shell to it,
    // so the file is polled for rather than assumed. Reading the recorded pid rather than
    // assuming a layout is what lets the failure below name the pid that survived.
    let pid = recorded_descendant_pid(&pid_file)?;

    // #722: the property under test is that nothing the timed-out `gh` started is still
    // *running* -- that the group kill released the pipes the drain threads were waiting on.
    // Whether the kernel has finished bookkeeping that pid is a different question, and the
    // difference is what this poll exists to express.
    //
    // A one-shot `kill -0` cannot express it. `kill(pid, 0)` runs only the existence and
    // permission checks, so it answers **successfully for a zombie**: a process that has been
    // killed and is merely waiting to be reaped still answers. The descendant's parent is the
    // stub script, and the gate kills and reaps that script itself, so the descendant is only
    // re-parented (and only finished reaping) after the gate's own `wait`. Sampling once inside
    // that window reported a surviving process on 4 of 20 full-suite runs against the fixed
    // code -- a coin flip on a required status check.
    //
    // So: poll to a deadline, and treat "exited" as good enough. `waitid(EXITED | NOHANG |
    // NOWAIT)` with `NOWAIT` is what distinguishes the two -- it reports a process that has
    // exited without consuming its status, so asking never steals the reap from a real parent.
    let deadline = std::time::Instant::now() + DESCENDANT_EXIT_TIMEOUT;
    loop {
        if !descendant_is_running(pid) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "the descendant {pid} is still running {DESCENDANT_EXIT_TIMEOUT:?} after the \
                 timeout killed its group, so only the direct child was killed and the work \
                 `gh` started outlived the bound the gate reported"
            )
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
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

/// Read the pid the stub recorded, polling because the deadline can beat the shell to it.
///
/// Without a recorded pid there is nothing to assert about, so this is where a stub that failed to
/// background anything is reported rather than silently passing the assertion below it.
#[cfg(unix)]
fn recorded_descendant_pid(pid_file: &std::path::Path) -> Result<i32, Box<dyn Error>> {
    let deadline = std::time::Instant::now() + DESCENDANT_EXIT_TIMEOUT;
    loop {
        if let Ok(recorded) = std::fs::read_to_string(pid_file)
            && let Ok(pid) = recorded.trim().parse::<i32>()
        {
            return Ok(pid);
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "the stub never recorded a descendant pid at {}",
                pid_file.display()
            )
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Report whether the descendant is still running, treating "exited" as gone.
///
/// #722: the group kill leaves the descendant a zombie until someone collects it, and a zombie
/// answers every existence probe, so "gone" has to be established rather than sampled. The
/// descendant's parent is the stub script, which the gate kills and then reaps in `reap()`; once
/// that happens the descendant is re-parented to init, which reaps it. This process is therefore
/// usually *not* its parent, which is why `waitid` is a query here rather than a wait: asking
/// about a process that is not this process's child fails with `ECHILD`, and that is a normal
/// answer, not an error to propagate.
#[cfg(unix)]
fn descendant_is_running(pid: i32) -> bool {
    let Some(pid) = rustix::process::Pid::from_raw(pid) else {
        // Not a usable pid, so it cannot be a running process.
        return false;
    };
    // `NOWAIT` leaves the status for a real parent to collect, so this never steals a reap.
    let options = rustix::process::WaitIdOptions::EXITED
        | rustix::process::WaitIdOptions::NOHANG
        | rustix::process::WaitIdOptions::NOWAIT;
    match rustix::process::waitid(rustix::process::WaitId::Pid(pid), options) {
        // A status is waiting: the process has exited, so whatever state it is in, it is not
        // running and it is not holding the pipes. This is the zombie case, and treating it as
        // gone is the whole point.
        Ok(Some(_status)) => false,
        // No status yet: still running.
        Ok(None) => true,
        // `ECHILD` means it is not this process's child, so there is no status here to wait for
        // and the pid has to be probed the other way. `kill(pid, 0)` can only over-report now --
        // a zombie would read as alive -- which is why this is the fallback and not the primary.
        Err(_) => matches!(rustix::process::test_kill_process(pid), Ok(())),
    }
}
