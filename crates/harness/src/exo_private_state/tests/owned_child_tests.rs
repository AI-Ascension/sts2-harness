// SPDX-License-Identifier: MIT
#![allow(clippy::expect_used)]

use super::OwnedChild;
use std::io;
use std::process::Command;
use std::time::Duration;

#[test]
fn timed_out_child_is_killed_reaped_and_keeps_captured_output() -> io::Result<()> {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("printf fixture-output; exec sleep 30");
    let mut child = OwnedChild::spawn(command)?;
    let error = child
        .wait_until(Duration::from_millis(500))
        .expect_err("the sleeping owned child must time out");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    let message = error.to_string();
    assert!(
        message.contains("child exit/reap was observed during timeout cleanup"),
        "timeout cleanup must report an observed direct-child reap: {message}"
    );
    assert!(
        message.contains("fixture-output"),
        "timeout diagnostics must retain bounded captured output: {message}"
    );
    Ok(())
}
