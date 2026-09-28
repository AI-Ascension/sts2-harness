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

/// #717: a timeout must not leave the work it started running behind the bound it reported.
///
/// The three tests above cannot see this defect, and the reason is worth stating rather than
/// leaving to be rediscovered. They all hang the stub's *own shell*, and killing the direct child
/// ends that shell -- so both timeout tests pass against an implementation that signals one pid and
/// leaves every descendant behind, which is exactly what this test exists to catch.
///
/// So the stub here is a *grandparent* one. It backgrounds a long `sleep`, records the pid it was
/// given, and then blocks itself, so there is a descendant that outlives the direct child. What
/// the assertion needs is that the recorded pid is gone once the call has returned.
#[cfg(unix)]
#[test]
fn a_timed_out_gh_leaves_no_surviving_descendant() -> Result<(), Box<dyn Error>> {
    // The stub records the descendant's pid in its own directory, beside the script itself. The
    // timeout can fire before the shell has finished backgrounding the sleep, so the file is
    // polled for rather than assumed: with no recorded pid there would be nothing to assert.
    let (guard, path) = grandparent_stub()?;
    let pid_file = guard.path.join("descendant.pid");
    let runner = GhRunner {
        gh_path: path.to_string_lossy().into_owned(),
        timeout: std::time::Duration::from_millis(200),
    };

    // The call itself must still be bounded. This is the property #702 bought and #717 must not
    // regress while closing the gap below it: the descendant is killed, not waited on.
    let started = std::time::Instant::now();
    let outcome = runner.head_sha("AI-Ascension/sts2-harness", 693);
    let elapsed = started.elapsed();
    assert!(
        matches!(&outcome, Err(ReviewGateError(message)) if message.contains("timed out")),
        "the grandparent stub must time out rather than yield a verdict, got {outcome:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the call took {elapsed:?}, so it waited on the child rather than on the deadline"
    );

    let raw_pid = recorded_descendant_pid(&pid_file)?;
    let pid = rustix::process::Pid::from_raw(raw_pid).ok_or_else(|| {
        format!("the stub recorded pid {raw_pid}, which is not a usable process id")
    })?;
    // The descendant must be *reaped*, not merely signalled. `kill(pid, 0)` reports a zombie as
    // alive, so a signal-only implementation fails here too, but the distinction this test turns
    // on is liveness: a descendant that is gone satisfies the pipe-holding concern whether or not
    // its parent had collected it, and the stub's own shell is killed rather than exited, so this
    // process is the one that would have to reap it and deliberately does not.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        // `test_kill_process` is the kernel's signal-0 probe: it runs the existence and permission
        // checks and sends nothing, which is exactly what "is it still there" means.
        match rustix::process::test_kill_process(pid) {
            // Alive. Give the SIGKILL a moment to land before deciding it did not.
            Ok(()) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Ok(()) => {
                return Err(format!(
                    "the descendant {raw_pid} spawned by the timed-out `gh` is still running, so the \
                     gate's call was bounded but the work it started was not: the timeout killed \
                     the direct child only"
                )
                .into());
            }
            Err(rustix::io::Errno::SRCH) => break,
            // EPERM would mean the pid exists and is not ours to signal, which is a failure of the
            // property rather than of the probe, so it is reported rather than treated as gone.
            Err(error) => {
                return Err(format!("unable to probe descendant {raw_pid}: {error}").into());
            }
        }
    }
    Ok(())
}

/// A stub that backgrounds a long `sleep`, records its pid, and then blocks itself.
///
/// The blocking `sleep` at the end is what makes the stub a *grandparent* rather than a parent:
/// the direct child the gate spawns and reaps is this shell, and the recorded pid belongs to a
/// process one level below it. Writing the pid before blocking means the descendant is already
/// running when the deadline expires, which is the state the timeout has to clean up.
#[cfg(unix)]
fn grandparent_stub() -> Result<(TempDir, PathBuf), Box<dyn Error>> {
    let directory = TempDir::new("grandparent")?;
    let path = directory.path.join("gh");
    let script = format!(
        "#!/bin/sh\n\
         sleep 3600 &\n\
         echo $! > {pid_file}\n\
         sleep 3600\n",
        pid_file = directory.path.join("descendant.pid").display(),
    );
    std::fs::write(&path, script)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok((directory, path))
}

/// Read the descendant pid the stub recorded, polling because the deadline can beat the shell.
#[cfg(unix)]
fn recorded_descendant_pid(path: &std::path::Path) -> Result<i32, Box<dyn Error>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(recorded) = std::fs::read_to_string(path)
            && let Ok(pid) = recorded.trim().parse::<i32>()
        {
            return Ok(pid);
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "the stub never recorded a descendant pid at {}",
                path.display()
            )
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
