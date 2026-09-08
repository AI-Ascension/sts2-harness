// SPDX-License-Identifier: MIT

use std::fs;

use serde_json::Value;

use super::*;

#[test]
fn expert_catalog_pipe_failures_recover_after_normal_catalog_without_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    for failure in ["expert-eof", "expert-timeout", "expert-gateway"] {
        let fixture = Fixture::new()?;
        let expected_reconnects = if failure == "expert-gateway" { 0 } else { 1 };
        run_runner_fixture(&fixture, failure, true, expected_reconnects, 1)?;
        let requests: Vec<Value> = fs::read_to_string(fixture.0.join("requests"))?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        let tool_calls: Vec<_> = requests
            .iter()
            .filter_map(|request| request["params"]["name"].as_str())
            .collect();
        assert_eq!(
            tool_calls,
            [
                "sts2.observe",
                "sts2.expert_state",
                "sts2.legal_actions",
                "sts2.expert_state",
                "sts2.reobserve",
                "sts2.expert_state"
            ]
        );
    }
    Ok(())
}

#[test]
fn expert_reobserve_transport_failures_are_bounded_reads_after_normal_reobserve()
-> Result<(), Box<dyn std::error::Error>> {
    for failure in [
        "expert-reobserve-eof",
        "expert-reobserve-timeout",
        "expert-reobserve-gateway",
    ] {
        let fixture = Fixture::new()?;
        let expected_reconnects = 1;
        run_runner_fixture(&fixture, failure, true, expected_reconnects, 2)?;
        let requests: Vec<Value> = fs::read_to_string(fixture.0.join("requests"))?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        let tool_calls: Vec<_> = requests
            .iter()
            .filter_map(|request| request["params"]["name"].as_str())
            .collect();
        assert_eq!(
            tool_calls,
            [
                "sts2.observe",
                "sts2.expert_state",
                "sts2.legal_actions",
                "sts2.reobserve",
                "sts2.expert_state",
                "sts2.reobserve",
                "sts2.expert_state",
            ]
        );
    }
    Ok(())
}

#[test]
fn expert_reobserve_schema_auth_and_identity_failures_are_terminal()
-> Result<(), Box<dyn std::error::Error>> {
    for failure in [
        "expert-reobserve-schema",
        "expert-reobserve-auth",
        "expert-reobserve-identity",
    ] {
        let fixture = Fixture::new()?;
        run_runner_fixture_terminal(&fixture, failure, 0)?;
    }
    Ok(())
}
