// SPDX-License-Identifier: MIT

//! Native worker endpoint for the owner-local watchdog handoff.
//!
//! The Linux implementation is split by boundary so bootstrap, peer proof,
//! transport, and execution code remain independently reviewable.

#[cfg(target_os = "linux")]
mod linux {
    include!("worker_endpoint/bootstrap.rs");
    include!("worker_endpoint/bootstrap_io.rs");
    include!("worker_endpoint/security.rs");
    include!("worker_endpoint/transport.rs");
    include!("worker_endpoint/transport_auth.rs");
    include!("worker_endpoint/execution.rs");
    include!("worker_endpoint/execution_support.rs");
    #[cfg(test)]
    include!("worker_endpoint/tests.rs");
}

#[cfg(target_os = "linux")]
pub use linux::run_from_environment;

#[cfg(not(target_os = "linux"))]
/// The native endpoint is deliberately unavailable until a platform adapter
/// can provide the equivalent protected peer/session contract.
pub fn run_from_environment() -> Result<(), String> {
    Err(String::from(
        "native worker endpoint requires the Linux Unix-peer adapter",
    ))
}
