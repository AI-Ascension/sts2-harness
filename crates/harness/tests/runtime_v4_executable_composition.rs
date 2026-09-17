// SPDX-License-Identifier: MIT

#![cfg(unix)]

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
mod fixture;
#[path = "support/runtime_v4_executable_composition_process.rs"]
mod process;

use fixture::FixtureMode;
use process::{
    TempDir, assert_foreign_state_rejected, assert_malformed_envelope_rejected, assert_success,
    executable, run_scenario, run_served_cancel_after_accepted_barrier,
    run_served_context_receipt_recovery, run_served_context_source_adoption,
    run_served_policy_gate, run_served_restart_refuses_duplicate_effect, write_evidence,
};

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn executable_runtime_v4_composes_unknown_reconcile_and_foreign_state_fence()
-> Result<(), Box<dyn std::error::Error>> {
    let gateway = executable("STS2_GATEWAY_BINARY")?;
    let mcp = executable("STS2_MCP_BINARY")?;
    let harness = executable("STS2_HARNESS_RUNTIME_BINARY")?;
    let temporary = TempDir::new()?;
    let bridge = temporary.bridge()?;

    let success = run_scenario(&gateway, &mcp, &harness, &bridge, FixtureMode::Success)?;
    let operation = assert_success(&success)?;
    let foreign = run_scenario(
        &gateway,
        &mcp,
        &harness,
        &bridge,
        FixtureMode::ForeignExpertState,
    )?;
    assert_foreign_state_rejected(&foreign)?;
    let malformed = run_scenario(
        &gateway,
        &mcp,
        &harness,
        &bridge,
        FixtureMode::MalformedExpertState,
    )?;
    assert_malformed_envelope_rejected(&malformed)?;
    write_evidence(&success, &foreign, &operation)
}

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn served_workflow_settles_action_with_adopted_provider_policy()
-> Result<(), Box<dyn std::error::Error>> {
    let gateway = executable("STS2_GATEWAY_BINARY")?;
    let mcp = executable("STS2_MCP_BINARY")?;
    let harness = executable("STS2_HARNESS_RUNTIME_BINARY")?;
    run_served_policy_gate(&gateway, &mcp, &harness)
}

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn served_restart_refuses_unknown_effect_without_redispatch()
-> Result<(), Box<dyn std::error::Error>> {
    run_served_restart_refuses_duplicate_effect(
        &executable("STS2_GATEWAY_BINARY")?,
        &executable("STS2_MCP_BINARY")?,
        &executable("STS2_HARNESS_RUNTIME_BINARY")?,
    )
}

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn served_cancel_reconciles_an_accepted_unsettled_action() -> Result<(), Box<dyn std::error::Error>>
{
    run_served_cancel_after_accepted_barrier(
        &executable("STS2_GATEWAY_BINARY")?,
        &executable("STS2_MCP_BINARY")?,
        &executable("STS2_HARNESS_RUNTIME_BINARY")?,
    )
}

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn served_workflow_adopts_context_source_before_managed_decision()
-> Result<(), Box<dyn std::error::Error>> {
    run_served_context_source_adoption(
        &executable("STS2_GATEWAY_BINARY")?,
        &executable("STS2_MCP_BINARY")?,
        &executable("STS2_HARNESS_RUNTIME_BINARY")?,
    )
}

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn served_workflow_recovers_context_receipt_after_process_restart()
-> Result<(), Box<dyn std::error::Error>> {
    run_served_context_receipt_recovery(
        &executable("STS2_GATEWAY_BINARY")?,
        &executable("STS2_MCP_BINARY")?,
        &executable("STS2_HARNESS_RUNTIME_BINARY")?,
    )
}
