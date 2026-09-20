// SPDX-License-Identifier: MIT

//! The host half of the host-lease-control sideband for the served fixture.
//!
//! `sts2-gateway` forwards every recovery mutation to the mod address over the
//! fixed `POST /api/v1/runtime/recovery` mux, so the synthetic downstream that a
//! soak campaign runs has to terminate that hop itself. This module resolves the
//! pinned `watchdog-host-lease-control-v1` terminal for that hop and keeps the
//! binding entry point next to it, so the shared fixture stays a transport.

#[path = "../host_lease_control_frames.rs"]
pub(crate) mod host_lease_control;

#[path = "../host_lease_control_canonical.rs"]
pub(crate) mod host_lease_control_canonical;

use std::sync::Arc;

use serde_json::Value;

use host_lease_control::HostLeaseControl;

impl super::ModServer {
    /// Bind the synthetic downstream with an in-process host lease-control
    /// terminal. Operator campaigns leave this empty and configure the same
    /// terminal through the environment instead.
    #[allow(
        dead_code,
        reason = "used by the host lease-control conformance target"
    )]
    pub(crate) fn bind_with_host_lease(
        address: &str,
        mode: super::FixtureMode,
        host_lease: Arc<HostLeaseControl>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::bind_inner(address, mode, None, Some(host_lease))
    }
}

/// Resolve the terminal for one recovery frame: the in-process terminal when the
/// caller supplied one, otherwise the operator's environment, otherwise a
/// refusal that names the missing variable. Without a key the mux stays closed
/// and the refusal is recorded in the downstream ledger instead of being
/// silently satisfied. An accepted install moves the identity the runtime
/// routes are fenced with, because the lease the gateway installs is the one it
/// tells the deployment to present.
pub(crate) fn recovery_response(
    raw: &[u8],
    host_lease: Option<&HostLeaseControl>,
    identity: &super::Identity,
) -> Result<(u16, Value), String> {
    let configured;
    let terminal = if let Some(terminal) = host_lease {
        terminal
    } else {
        configured = HostLeaseControl::from_env()?.ok_or_else(|| {
            String::from(
                "the recovery mux is closed: STS2_SYNTHETIC_HOST_LEASE_KEY is not configured",
            )
        })?;
        &configured
    };
    let response = terminal.respond(raw)?;
    identity.adopt_install(raw);
    Ok((200, response))
}
