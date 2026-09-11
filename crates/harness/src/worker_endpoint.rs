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
    type EndpointListener = UnixListener;
    type EndpointStream = UnixStream;
    include!("worker_endpoint/listener.rs");
    include!("worker_endpoint/execution.rs");
    include!("worker_endpoint/execution_support.rs");
    #[cfg(test)]
    include!("worker_endpoint/tests.rs");
}

#[cfg(target_os = "linux")]
pub use linux::run_from_environment;

#[cfg(windows)]
mod windows {
    include!("worker_endpoint/windows/bootstrap.rs");
    include!("worker_endpoint/windows/bootstrap_io.rs");
    include!("worker_endpoint/windows/security.rs");
    include!("worker_endpoint/windows/transport.rs");
    include!("worker_endpoint/windows/transport_auth.rs");
    type EndpointListener = sts2_harness_windows_boundary::WorkerPipeListener;
    type EndpointStream = sts2_harness_windows_boundary::WorkerPipeStream;
    include!("worker_endpoint/execution.rs");
    include!("worker_endpoint/windows/execution_support.rs");
    #[cfg(test)]
    include!("worker_endpoint/windows/tests.rs");
}

#[cfg(windows)]
pub use windows::run_from_environment;

#[cfg(not(any(target_os = "linux", windows)))]
/// The native endpoint is deliberately unavailable until a platform adapter
/// can provide the equivalent protected peer/session contract.
pub fn run_from_environment() -> Result<(), String> {
    Err(String::from(
        "native worker endpoint requires the Linux Unix-peer adapter",
    ))
}
