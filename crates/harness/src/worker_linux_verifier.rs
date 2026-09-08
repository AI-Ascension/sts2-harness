// SPDX-License-Identifier: MIT

//! Private Linux peer-proof verifier module facade.
//!
//! The controller lease, child cleanup authority, fixed wire schema, helper
//! proof, and descriptor transport are kept in dedicated modules.

#![cfg(target_os = "linux")]

use super::LinuxTransportError;

#[cfg(test)]
const TEST_HELPER_ENV: &str = "ASCENSION_TEST_WORKER_VERIFIER_HELPER";
#[cfg(test)]
const TEST_HELPER_NAME: &str =
    "worker_local_linux::worker_linux_verifier::tests::verifier_process_entry";

#[path = "worker_linux_verifier_controller.rs"]
mod controller;
#[path = "worker_linux_verifier_helper.rs"]
mod helper;
#[path = "worker_linux_verifier_helper_io.rs"]
mod helper_io;
#[path = "worker_linux_verifier_lifecycle.rs"]
mod lifecycle;
#[path = "worker_linux_verifier_protocol.rs"]
mod protocol;
#[path = "worker_linux_verifier_transport.rs"]
mod transport;

pub(super) use controller::VerifierController;
use helper::run_verifier_from_stdin as run_verifier_process;
pub(super) use protocol::{VerifierFailure, VerifierOutcome};

/// Internal implementation behind the public fixed executable entry point.
pub(crate) fn run_verifier_from_stdin() -> Result<(), LinuxTransportError> {
    run_verifier_process()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_process_entry() {
        if std::env::var_os(TEST_HELPER_ENV).is_some() {
            assert!(super::super::run_peer_verifier_from_stdin().is_ok());
        }
    }
}
