// SPDX-License-Identifier: MIT

#![cfg(unix)]

#[path = "support/runtime_v4_rest_executable_composition_fixture.rs"]
mod fixture;
#[path = "support/runtime_v4_rest_executable_composition_process.rs"]
mod process;

use process::{
    TempDir, assert_success, assert_success_with_style, executable, run_scenario,
    run_scenario_with_style, write_evidence,
};

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn executable_runtime_v4_rest_composes_full_selection_recovery_chain()
-> Result<(), Box<dyn std::error::Error>> {
    fixture::verify_canonical_inputs()?;
    let gateway = executable("STS2_GATEWAY_BINARY")?;
    let mcp = executable("STS2_MCP_BINARY")?;
    let harness = executable("STS2_HARNESS_RUNTIME_BINARY")?;
    let temporary = TempDir::new()?;
    let bridge = temporary.bridge()?;
    let result = run_scenario(&gateway, &mcp, &harness, &bridge)?;
    let operation = assert_success(&result)?;
    write_evidence(&result, &operation)
}

#[test]
#[ignore = "operator-only test; requires explicitly built gateway, MCP, and harness binaries"]
fn executable_runtime_v4_rest_composes_native_selector_id_chain()
-> Result<(), Box<dyn std::error::Error>> {
    fixture::verify_canonical_inputs()?;
    let gateway = executable("STS2_GATEWAY_BINARY")?;
    let mcp = executable("STS2_MCP_BINARY")?;
    let harness = executable("STS2_HARNESS_RUNTIME_BINARY")?;
    let temporary = TempDir::new()?;
    let bridge = temporary.bridge_native()?;
    let result = run_scenario_with_style(
        &gateway,
        &mcp,
        &harness,
        &bridge,
        fixture::SelectorEncoding::Native,
    )?;
    let operation = assert_success_with_style(&result, fixture::SelectorEncoding::Native)?;
    write_evidence(&result, &operation)
}
