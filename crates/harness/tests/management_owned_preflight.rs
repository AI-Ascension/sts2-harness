// SPDX-License-Identifier: MIT

//! The real commands refuse inherited auto-reaping before opening owner state or spawning.
//! Bash changes SIGCHLD only in an isolated process; the test runner's disposition is untouched.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

struct IsolatedCommand {
    child: Child,
    root: PathBuf,
}

impl Drop for IsolatedCommand {
    fn drop(&mut self) {
        // Only kill this fixture's group while its unreaped leader still owns its PID.
        // This also cleans up helpers if a regression creates them before refusing startup.
        if self.child.try_wait().is_ok_and(|status| status.is_none())
            && let Some(group) = Pid::from_raw(self.child.id() as i32)
        {
            let _ = kill_process_group(group, Signal::KILL);
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn command(executable: &str, runtime: bool, root: &Path) -> Command {
    let store = root.join("workflow.sqlite");
    let mut command = Command::new("/bin/bash");
    command
        .args(["-c", "trap '' CHLD; exec \"$@\"", "owned-child-wait-test"])
        .arg(executable)
        .env_clear()
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            fs::File::create(root.join("stderr")).expect("bounded startup diagnostics"),
        ));
    if runtime {
        command
            .arg("serve-workflow")
            .env("STS2_WORKFLOW_LISTEN", "127.0.0.1:0")
            .env("STS2_WORKFLOW_STORE", &store)
            .env("STS2_WORKFLOW_AUTH_PROFILE", "child-wait");
    } else {
        command
            .args(["serve", "--listen", "127.0.0.1:0", "--store"])
            .arg(&store)
            .args(["--auth-profile", "child-wait"]);
    }
    command
}

fn assert_refused(executable: &str, runtime: bool) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/owned-preflight-tests")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).expect("isolated preflight directory");
    let child = command(executable, runtime, &root)
        .spawn()
        .expect("isolated shipped command");
    let mut isolated = IsolatedCommand { child, root };
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = isolated.child.try_wait().expect("owned command exit") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "incompatible child waiting must refuse promptly"
        );
        thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(status.code(), Some(2));
    let diagnostic = fs::read_to_string(isolated.root.join("stderr")).expect("startup refusal");
    assert!(diagnostic.contains("SIGCHLD is ignored"), "{diagnostic}");
    assert!(
        !isolated.root.join("workflow.sqlite").exists(),
        "preflight must precede owner-store composition",
    );
}

#[test]
fn workflow_command_refuses_inherited_ignored_sigchld() {
    assert_refused(env!("CARGO_BIN_EXE_sts2-workflow"), false);
}

#[test]
fn runtime_command_refuses_inherited_ignored_sigchld() {
    assert_refused(env!("CARGO_BIN_EXE_sts2-harness-runtime"), true);
}
