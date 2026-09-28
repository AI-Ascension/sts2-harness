// SPDX-License-Identifier: MIT

//! Every path that spawns the served gateway, including the one that hands a child an address
//! this test does not own. Refs #673.
//!
//! Split into its own module so the parent stays inside the repository's preferred test-file
//! size budget, exactly as `gateway_capture.rs` was.

use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Stdio};

use super::super::fixture::{
    CALLER_ID, INSTANCE_ID, LEASE_EPOCH, LEASE_ID, MCP_SESSION_ID, SESSION_ID,
};
use super::{GatewayProcess, ModAddress, ReservedAddress, release_onto_gateway, spawn};

/// Spawn the served gateway on a **reserved** address. See [`super::loopback_address`].
pub(crate) fn gateway(
    binary: &Path,
    address: &ReservedAddress,
    mod_address: ModAddress,
) -> Result<GatewayProcess, Box<dyn std::error::Error>> {
    gateway_with_identity(
        binary,
        address,
        mod_address,
        INSTANCE_ID,
        LEASE_ID,
        LEASE_EPOCH,
    )
}

pub(crate) fn gateway_with_identity(
    binary: &Path,
    address: &ReservedAddress,
    mod_address: ModAddress,
    instance_id: &str,
    lease_id: &str,
    lease_epoch: u64,
) -> Result<GatewayProcess, Box<dyn std::error::Error>> {
    let mut command = gateway_command(binary, instance_id, lease_id, lease_epoch);
    // The reservations are given up here, immediately before the spawn, and not a moment
    // earlier: every microsecond still holding them is a microsecond of the #673 defect.
    release_onto_gateway(&mut command, address, &mod_address);
    Ok(GatewayProcess::attach(spawn::retrying_text_busy(
        &mut command,
    )?)?)
}

/// The cleared environment, piped streams, and process group every served gateway is spawned
/// with, before either address is named.
pub(crate) fn gateway_command(
    binary: &Path,
    instance_id: &str,
    lease_id: &str,
    lease_epoch: u64,
) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("STS2_GATEWAY_TOKEN", "gateway-token")
        .env("STS2_MOD_TOKEN", "mod-token")
        .env("STS2_INSTANCE_ID", instance_id)
        .env("STS2_CALLER_ID", CALLER_ID)
        .env("STS2_SESSION_ID", SESSION_ID)
        .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
        .env("STS2_LEASE_ID", lease_id)
        .env("STS2_LEASE_EPOCH", lease_epoch.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    // The pipes are taken and drained here, at spawn, rather than after the gateway has been
    // killed. That is the whole fix for #559: a pipe holds one buffer before the writer blocks,
    // so reading only at `stop` truncated a chatty gateway at one buffer and silently lost the
    // rest. `GatewayProcess::attach` also owns the failure path, so a gateway that is spawned
    // but cannot be captured is killed here rather than leaked.
    command
}

/// Spawn the served gateway on an address **something else is already serving**.
///
/// This is the one deliberate escape from [`super::super::gateway`], and it exists only for the
/// squatter scenario in `served_gateway_capture_drain.rs` (`#673`, pinned by `#689`). That
/// scenario has to *arrange* a squatter to prove the harness never reads a foreign listener as a
/// quiet gateway, so it cannot hold the gateway address the way every other scenario does —
/// holding it is precisely what makes a squatter unreachable, and that is the behaviour the
/// scenario is deliberately bypassing in order to observe what the harness reports instead.
///
/// It is a named function rather than a second `ModAddress`-shaped enum variant for two
/// reasons. The reservation is the thing being tested, so nothing else in this module may hold
/// a bare `SocketAddr` without a name explaining which scenario wanted it; and the mod address
/// is still a real reservation, so the call keeps the ordinary [`ModAddress`] and only the
/// gateway address is the foreign one. Callers that are not that scenario should use
/// [`super::super::gateway`].
pub(crate) fn spawn_on_squatted_address(
    binary: &Path,
    address: SocketAddr,
    mod_address: ModAddress,
) -> Result<GatewayProcess, Box<dyn std::error::Error>> {
    let mut command = gateway_command(binary, INSTANCE_ID, LEASE_ID, LEASE_EPOCH);
    // The mod address is still ours, so it is still released here and only here — a squatted
    // *gateway* address must not become a squatted mod address as a side effect.
    command.env("STS2_GATEWAY_ADDR", address.to_string());
    command.env("STS2_MOD_ADDR", mod_address.release().to_string());
    Ok(GatewayProcess::attach(spawn::retrying_text_busy(
        &mut command,
    )?)?)
}
