// SPDX-License-Identifier: MIT

#![cfg(unix)]

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
mod fixture;
#[path = "support/runtime_v4_executable_composition_process.rs"]
mod process;

use fixture::FixtureMode;
use process::{
    TempDir, assert_foreign_state_rejected, assert_success, executable, run_scenario,
    write_evidence,
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
    write_evidence(&success, &foreign, &operation)
}
