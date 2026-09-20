// SPDX-License-Identifier: MIT
//! Operator-only long-lived synthetic downstream for soak campaigns.
//!
//! Runs the deterministic fake mod server (the same test-support fixture the
//! executable composition test uses) until the process is terminated, so a
//! supervisor can keep the gateway, MCP, and harness runtime running against a
//! stable synthetic downstream for an extended campaign. This is test support:
//! it produces no real game behavior and grants no authority.

#![cfg(unix)]

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
#[allow(
    dead_code,
    reason = "this operator target uses a subset of the shared fixture"
)]
mod fixture;

use fixture::{FixtureMode, ModServer};
use std::time::Duration;

#[test]
#[ignore = "operator-only: runs a long-lived synthetic downstream mod server"]
fn run_synthetic_downstream_until_terminated() -> Result<(), Box<dyn std::error::Error>> {
    let address =
        std::env::var("STS2_SYNTHETIC_MOD_ADDR").unwrap_or_else(|_| "127.0.0.1:0".to_owned());
    let mode = match std::env::var("STS2_SYNTHETIC_MOD_MODE").as_deref() {
        Ok("foreign-state") => FixtureMode::ForeignExpertState,
        _ => FixtureMode::Success,
    };
    // The gateway drives every recovery mutation at this address, so the
    // operator has to supply the pinned host lease-control configuration for
    // the durable recovery path to have a host to talk to.
    let host_lease = match fixture::host_lease_mux::host_lease_control::HostLeaseControl::from_env()
    {
        Ok(Some(_)) => "enabled",
        Ok(None) => "closed",
        Err(error) => return Err(error.into()),
    };
    let server = ModServer::bind(&address, mode)?;
    println!(
        "synthetic_mod_listening={} mode={:?} host_lease={host_lease}",
        server.address,
        std::env::var("STS2_SYNTHETIC_MOD_MODE").unwrap_or_else(|_| "success".to_owned())
    );
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
