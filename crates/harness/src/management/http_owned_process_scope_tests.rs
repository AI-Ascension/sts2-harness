// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

const ENTRY: &str = "management::http::owned_process_scope::tests::isolated_scope_entry";
const ROLE: &str = "STS2_MANAGEMENT_SCOPE_TEST_ROLE";
const RECORD: &str = "STS2_MANAGEMENT_SCOPE_TEST_RECORD";

struct OwnedTestChild(Child);

impl Drop for OwnedTestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn child(role: &str) -> io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", ENTRY, "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    Ok(command)
}

fn run_case(case: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/owned-scope-tests")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&output)?;
    let log = output.join("stderr");
    let mut isolated = OwnedTestChild(child(case)?.stderr(fs::File::create(&log)?).spawn()?);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = isolated.0.try_wait()? {
            break status;
        }
        assert!(Instant::now() < deadline, "isolated scope driver deadline");
        thread::sleep(Duration::from_millis(1));
    };
    let mut diagnostic = String::new();
    fs::File::open(log)?
        .take(65_536)
        .read_to_string(&mut diagnostic)?;
    assert!(
        status.success(),
        "isolated scope case {case} failed: {diagnostic}",
    );
    fs::remove_dir_all(output)?;
    Ok(())
}

#[test]
fn termination_reaps_a_noncooperative_backend_and_escaped_descendant()
-> Result<(), Box<dyn std::error::Error>> {
    run_case("tree")
}

#[test]
fn startup_refuses_a_preexisting_child_without_signalling_or_reaping_it()
-> Result<(), Box<dyn std::error::Error>> {
    run_case("existing")
}

#[test]
fn isolated_scope_entry() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(role) = std::env::var(ROLE) else {
        return Ok(());
    };
    match role.as_str() {
        "tree" => terminate_tree(),
        "existing" => refuse_existing(),
        "backend" => {
            let descendant = child("leaf")?.process_group(0).spawn()?;
            let record = PathBuf::from(std::env::var(RECORD)?);
            let pending = record.with_extension("pending");
            fs::write(&pending, descendant.id().to_string())?;
            fs::rename(pending, record)?;
            // No destructor, timeout, cancellation callback or IPC read can complete this
            // handler. The parent must terminate its process and drain the escaped child.
            loop {
                std::thread::park();
            }
        }
        "leaf" => loop {
            std::thread::park();
        },
        _ => Err("unknown isolated scope role".into()),
    }
}

fn terminate_tree() -> Result<(), Box<dyn std::error::Error>> {
    let scope = ChildScope::enable().map_err(|e| format!("enable: {e:?}"))?;
    assert!(matches!(
        ChildScope::enable(),
        Err(ScopeError::AlreadyOwned)
    ));
    let record: PathBuf = std::env::temp_dir().join(format!(
        "sts2-management-owned-child-{}",
        uuid::Uuid::new_v4()
    ));
    let mut backend = OwnedTestChild(child("backend")?.env(RECORD, &record).spawn()?);
    let ready_deadline = Instant::now() + Duration::from_secs(3);
    while !record.exists() {
        if Instant::now() >= ready_deadline {
            let _ = scope.terminate(&mut backend.0, Instant::now() + Duration::from_secs(2));
            return Err("backend did not record its owned descendant".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let retained = (|| -> Result<_, Box<dyn std::error::Error>> {
        let descendant_pid: i32 = fs::read_to_string(&record)?.parse()?;
        let descendant = Pid::from_raw(descendant_pid).ok_or("invalid descendant PID")?;
        Ok(pidfd_open(descendant, PidfdFlags::empty())?)
    })();
    let before = Instant::now();
    let result = scope.terminate(&mut backend.0, before + Duration::from_secs(2));
    let _ = fs::remove_file(&record);
    let retained = retained?;
    assert_eq!(result, Ok(()), "real child tree was reaped");
    assert!(before.elapsed() < Duration::from_secs(3));
    assert!(
        backend.0.try_wait()?.is_some(),
        "direct backend exit was cached"
    );
    assert!(
        matches!(
            waitid(
                WaitId::All,
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            ),
            Err(Errno::CHILD)
        ),
        "no owned child remains, including the different-process-group descendant"
    );
    assert_eq!(
        pidfd_send_signal(&retained, Signal::KILL),
        Err(Errno::SRCH),
        "the original descendant, not a reused numeric PID, has exited"
    );
    Ok(())
}

fn refuse_existing() -> Result<(), Box<dyn std::error::Error>> {
    let mut existing = OwnedTestChild(child("leaf")?.spawn()?);
    assert!(matches!(
        ChildScope::enable(),
        Err(ScopeError::ExistingChild)
    ));
    assert!(
        existing.0.try_wait()?.is_none(),
        "preexisting child was untouched"
    );
    existing.0.kill()?;
    existing.0.wait()?;
    Ok(())
}
