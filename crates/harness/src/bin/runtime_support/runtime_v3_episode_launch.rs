// SPDX-License-Identifier: MIT

use super::super::super::mcp::validate_or_release_allocation_with;
use super::*;
use std::collections::BTreeMap;

pub(super) fn launch(port: &mut RuntimeV3Port) -> Result<(), sts2_harness::PortError> {
    if port.allocated {
        if port.continuation_prelaunched {
            port.continuation_prelaunched = false;
            return Ok(());
        }
        return Err(wire::port_error(
            "duplicate_launch",
            "episode is already allocated",
            false,
        ));
    }
    // Allocation may commit even when its response is lost. Own cleanup before sending.
    port.allocated = true;
    let allocation = port.gateway.request(
        "POST",
        "/v1/sessions/allocate",
        &json!({
            "instance_id": port.config.instance_id,
            "caller_id": port.config.caller_id,
            "session_id": port.config.session_id
        }),
        BTreeMap::from([(
            String::from("x-mcp-session-id"),
            port.config.mcp_session_id.clone(),
        )]),
    );
    let code = if allocation.is_err() {
        "gateway_allocate_failed"
    } else {
        "gateway_allocate_invalid"
    };
    let allocation = validate_or_release_allocation_with(
        allocation,
        &port.config,
        allocation_context::validate,
        |headers| {
            let response = port.gateway.request(
                "POST",
                &format!("/v1/instances/{}/release", port.config.instance_id),
                &json!({}),
                headers,
            );
            port.released = response
                .as_ref()
                .is_ok_and(|value| value["status"] == "released");
            response
        },
    )
    .map_err(|error| wire::port_error(code, error, false))?;
    allocation.apply_current_lease(&mut port.config);
    port.require_lifecycle_lease_authority()?;
    port.recovery_authority = allocation.recovery_authority;
    if let Some(authority) = port.recovery_authority.as_ref() {
        port.recovery_context = Some(
            super::super::recovery::RecoveryContext::from_authority(authority, &port.config)
                .map_err(|error| wire::port_error("recovery_authority_invalid", error, false))?,
        );
    }
    if let Some(context) = port.continuation_owner_claim.clone()
        && let Err(error) = super::super::continuation_owner::claim_current_owner(
            &port.config,
            port.recovery_authority.as_ref(),
            &context,
        )
    {
        let release = port.release_lease_inner();
        return Err(wire::port_error(
            "continuation_owner_claim_failed",
            wire::combine_cleanup(error, Ok(()), release),
            false,
        ));
    }
    if port.exact_restore_selected {
        return Ok(());
    }
    if let Err(error) = port.launch_mcp() {
        return Err(wire::port_error("runtime_launch_failed", error, false));
    }
    if let Err(error) = port.reconcile_pending_operations() {
        let close = port.close_mcp_processes();
        let release = port.release_lease_inner();
        return Err(wire::port_error(
            "runtime_resume_failed",
            wire::combine_cleanup(error, close, release),
            false,
        ));
    }
    if port.config.seed_transport.is_some() {
        if let Err(error) = port.prime_seed_generation() {
            let close = port.close_mcp_processes();
            let release = port.release_lease_inner();
            return Err(wire::port_error(
                "seeded_run_preflight_failed",
                wire::combine_cleanup(error, close, release),
                false,
            ));
        }
        if let Err(error) = port.launch_seeded_run() {
            let close = port.close_mcp_processes();
            let release = port.release_lease_inner();
            return Err(wire::port_error(
                "seeded_run_failed",
                wire::combine_cleanup(error, close, release),
                false,
            ));
        }
    }
    Ok(())
}
