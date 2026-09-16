// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::{RuntimeConfig, parse_flag};

#[test]
fn map_context_flag_defaults_off_and_accepts_only_exact_booleans() {
    assert_eq!(parse_flag("X", "false"), Ok(false));
    assert_eq!(parse_flag("X", "true"), Ok(true));
    for value in ["", "1", "0", "yes", "TRUE", "True", " true"] {
        assert!(
            parse_flag("X", value).is_err(),
            "{value:?} must be rejected"
        );
    }
}

#[test]
fn branch_selector_requires_both_explicit_identity_components() {
    assert!(
        RuntimeConfig::selector_from_values(None, None)
            .expect("empty selector")
            .is_none()
    );
    assert!(
        RuntimeConfig::selector_from_values(Some("experiment:e".into()), None)
            .expect_err("partial selector")
            .contains("must be set together")
    );
    assert!(
        RuntimeConfig::selector_from_values(None, Some("branch:b".into()))
            .expect_err("partial selector")
            .contains("must be set together")
    );
    let selector =
        RuntimeConfig::selector_from_values(Some("experiment:e".into()), Some("branch:b".into()))
            .expect("explicit selector")
            .expect("selector exists");
    assert_eq!(selector.experiment_id(), "experiment:e");
    assert_eq!(selector.branch_id(), "branch:b");
}

#[test]
fn runtime_sessions_are_validated_independently() {
    let mut config = RuntimeConfig {
        seed_transport: None,
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-token"),
        mcp_binary: String::from("mcp"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("gateway-session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        mcp_session_id: String::from("mcp-session-independent"),
        run_id: "run-1".into(),
        episode_id: "episode-1".into(),
        trajectory_id: "trajectory-1".into(),
        trace_id: "trace-1".into(),
        artifact_id: "artifact-1".into(),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: Vec::new(),
    };
    assert!(config.validate().is_ok());
    config.mcp_session_id = String::from("unsafe session");
    assert!(config.validate().is_err());
    config.mcp_session_id = String::from("mcp-session-independent");
    config.session_id.clear();
    assert!(config.validate().is_err());
}
