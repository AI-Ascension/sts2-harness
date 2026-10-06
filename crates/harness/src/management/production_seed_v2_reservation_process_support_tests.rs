// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use super::{
    BARRIER, CATALOG, CHILD_FILTER, ChildReport, DATABASE, KEYRING, MODE, PROCESS_TIMEOUT, REQUEST,
    RESPONSE, ROLE,
};
pub(super) struct OwnedChild {
    pub(super) process_id: u32,
    child: Option<Child>,
}

pub(super) struct ChildOutput {
    pub(super) process_id: u32,
    pub(super) output: Output,
}

impl OwnedChild {
    pub(super) fn spawn(
        role: &str,
        database: &Path,
        request: &Path,
        response: &Path,
        keyring: &Path,
        barrier: &Path,
    ) -> std::io::Result<Self> {
        let child = Command::new(std::env::current_exe()?)
            .arg(CHILD_FILTER)
            .arg("--nocapture")
            .env(MODE, "race")
            .env(ROLE, role)
            .env(DATABASE, database)
            .env(REQUEST, request)
            .env(RESPONSE, response)
            .env(KEYRING, keyring)
            .env(CATALOG, "live.catalog.rotation-race")
            .env(BARRIER, barrier)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Self {
            process_id: child.id(),
            child: Some(child),
        })
    }

    pub(super) fn finish(mut self) -> Result<ChildOutput, String> {
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            match self
                .child
                .as_mut()
                .expect("child remains owned until wait completes")
                .try_wait()
            {
                Ok(Some(_)) => {
                    let output = self
                        .child
                        .take()
                        .expect("exited child remains owned")
                        .wait_with_output()
                        .map_err(|error| {
                            format!("collect child {} output: {error}", self.process_id)
                        })?;
                    return Ok(ChildOutput {
                        process_id: self.process_id,
                        output,
                    });
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => return Err(self.terminate("child exceeded its 30s deadline")),
                Err(error) => {
                    return Err(self.terminate(&format!("cannot observe child: {error}")));
                }
            }
        }
    }
}

impl OwnedChild {
    fn terminate(&mut self, reason: &str) -> String {
        let Some(mut child) = self.child.take() else {
            return format!("child {} {reason}", self.process_id);
        };
        let _ = child.kill();
        match child.wait_with_output() {
            Ok(output) => format!(
                "child {} {reason}; status={:?}; stdout={}; stderr={}",
                self.process_id,
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) => format!("child {} {reason}; reap failed: {error}", self.process_id),
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub(super) fn wait_for_files(paths: &[PathBuf], deadline: Instant) -> Result<(), String> {
    while !paths.iter().all(|path| path.is_file()) {
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for key selections {paths:?}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

pub(super) fn assert_child_success(child: &ChildOutput, label: &str) {
    assert!(
        child.output.status.success(),
        "{label} child {} failed with {:?}; stdout={}; stderr={}",
        child.process_id,
        child.output.status.code(),
        String::from_utf8_lossy(&child.output.stdout),
        String::from_utf8_lossy(&child.output.stderr)
    );
    let stdout = String::from_utf8_lossy(&child.output.stdout);
    assert!(
        stdout.lines().any(|line| line == "running 1 test"),
        "{label} child {} did not report one selected test: {stdout}",
        child.process_id
    );
    let selected_test = stdout
        .lines()
        .filter(|line| line.contains("::seed_v2_reservation_process_child_entry ... ok"))
        .count();
    assert_eq!(
        selected_test, 1,
        "{label} child {} did not pass the reservation child entry exactly once: {stdout}",
        child.process_id
    );
    let summaries = stdout
        .lines()
        .filter(|line| line.starts_with("test result:"))
        .collect::<Vec<_>>();
    assert!(
        summaries.len() == 1
            && summaries[0].starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;"),
        "{label} child {} did not report one passing, non-ignored test: {stdout}",
        child.process_id
    );
    eprintln!(
        "{label} child {} stdout={stdout}; stderr={}",
        child.process_id,
        String::from_utf8_lossy(&child.output.stderr)
    );
}

pub(super) fn read_report(path: &Path) -> ChildReport {
    serde_json::from_slice(&fs::read(path).expect("read child process report"))
        .expect("decode child process report")
}

pub(super) fn row_count(store: &crate::management::SqliteWorkflowStore, table: &str) -> i64 {
    assert!(matches!(
        table,
        "management_seed_operations"
            | "management_seed_bindings"
            | "management_submissions"
            | "management_runs"
            | "management_events"
    ));
    store
        .connection
        .lock()
        .expect("SQLite connection lock")
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("read durable row count")
}

pub(super) fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing child process input {name}"))
}

#[cfg(target_os = "linux")]
pub(super) fn set_private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .expect("protect interprocess barrier directory");
}

#[cfg(not(target_os = "linux"))]
pub(super) fn set_private_directory(_path: &Path) {}
