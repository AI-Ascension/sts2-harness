// SPDX-License-Identifier: MIT

//! Exact self-reexec for the private Linux verifier helper.

use std::os::fd::OwnedFd;
use std::process::{Child, Command, Stdio};

use super::protocol::VerifierFailure;

pub(super) fn spawn_fixed_verifier(control: OwnedFd) -> Result<Child, VerifierFailure> {
    // `/proc/self/exe` is resolved by the kernel against this process's
    // executable file. `current_exe()` instead returns the deleted memfd
    // name when the launcher started us from a sealed executable snapshot.
    // That name is not reopenable, while this procfs handle remains bound to
    // the exact running image for the verifier re-exec.
    let mut command = Command::new("/proc/self/exe");
    // The verifier consumes only its inherited control descriptor; it must
    // not receive runtime/provider credentials or configuration environment.
    command.env_clear();
    #[cfg(test)]
    command
        .args(["--exact", super::TEST_HELPER_NAME, "--nocapture"])
        .env(super::TEST_HELPER_ENV, "1");
    #[cfg(not(test))]
    command.arg("--worker-peer-verifier-v1");
    command
        .stdin(Stdio::from(std::fs::File::from(control)))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map_err(|_| VerifierFailure::Io)
}

#[cfg(test)]
#[path = "worker_linux_verifier_controller_tests.rs"]
mod tests;
