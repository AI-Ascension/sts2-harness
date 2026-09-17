// SPDX-License-Identifier: MIT

use super::*;

fn config(episode_profile: bool, lease_epoch: u64) -> RuntimeConfig {
    RuntimeConfig {
        seed_transport: None,
        gateway_address: "127.0.0.1:1".into(),
        gateway_token: "synthetic-token".into(),
        mcp_binary: "unused-test-binary".into(),
        runtime_profile: "runtime-v3-gameplay".into(),
        instance_id: "instance-1".into(),
        caller_id: "harness".into(),
        session_id: "session-1".into(),
        lease_id: "lease-1".into(),
        lease_epoch,
        episode_profile,
        mcp_session_id: "mcp-session-1".into(),
        run_id: "run-1".into(),
        episode_id: "episode-1".into(),
        trajectory_id: "trajectory-1".into(),
        trace_id: "trace-1".into(),
        artifact_id: "artifact-1".into(),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: Vec::new(),
    }
}

fn witness() -> Value {
    json!({
        "status": "released",
        "episode_profile": {
            "profile": EPISODE_PROFILE_NAME,
            "capability": EPISODE_PROFILE_CAPABILITY,
            "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST,
            "released_epoch": 3,
        }
    })
}

#[test]
fn lease_cleanup_requires_authoritative_release_confirmation() {
    let plain = config(false, 3);
    assert!(confirm_release(Ok(json!({"status":"released"})), &plain, false).is_ok());
    for response in [json!({}), json!({"status":"allocated"}), Value::Null] {
        assert_eq!(
            confirm_release(Ok(response), &plain, false),
            Err("gateway did not confirm lease release".into())
        );
    }
    assert_eq!(
        confirm_release(Err("unavailable".into()), &plain, false),
        Err("unavailable".into())
    );
}

/// A run that did not opt in must not require the gateway witness, so an
/// ordinary release keeps the legacy single-episode behavior byte-identical.
#[test]
fn an_unprofiled_release_needs_no_episode_witness() {
    let plain = config(false, 3);
    assert!(confirm_release(Ok(json!({"status":"released"})), &plain, true).is_ok());
    assert!(confirm_release(Ok(witness()), &plain, true).is_ok());
}

/// An episode-completing release that opted in must carry the exact accepted
/// witness; a gateway that silently dropped it would strand the next episode.
#[test]
fn an_episode_completing_release_requires_the_accepted_witness() {
    let profiled = config(true, 3);
    assert!(confirm_release(Ok(witness()), &profiled, true).is_ok());
    assert_eq!(
        confirm_release(Ok(json!({"status":"released"})), &profiled, true),
        Err("gateway release did not report the negotiated episode profile".into())
    );
    let wrong_epoch = json!({
        "status": "released",
        "episode_profile": {
            "profile": EPISODE_PROFILE_NAME,
            "capability": EPISODE_PROFILE_CAPABILITY,
            "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST,
            "released_epoch": 4,
        }
    });
    assert_eq!(
        confirm_release(Ok(wrong_epoch), &profiled, true),
        Err("gateway release reported an unexpected completed episode epoch".into())
    );
}

/// Only a release that completes an episode may arm the profile. Cleanup and
/// failure paths must keep the gateway's permanent-revocation default even when
/// the operator opted in.
#[test]
fn a_cleanup_release_never_arms_the_profile() {
    let profiled = config(true, 3);
    assert!(confirm_release(Ok(json!({"status":"released"})), &profiled, false).is_ok());
    assert!(
        confirm_release(Ok(witness()), &profiled, false).is_ok(),
        "an unnegotiated release must not be validated against the witness"
    );
    let headers = release_headers(&profiled, false);
    assert!(
        !headers.contains_key(EPISODE_PROFILE_HEADER),
        "a cleanup release must not send the profile header"
    );
    let armed = release_headers(&profiled, true);
    assert_eq!(
        armed.get(EPISODE_PROFILE_HEADER).map(String::as_str),
        Some(EPISODE_PROFILE_NAME)
    );
}

#[test]
fn raw_mcp_failures_and_unknown_observation_fields_are_not_logged() {
    let response = json!({"result":{"isError":true,"content":[{"text":"secret-marker"}]}});
    assert_eq!(
        require_success(&response, "get_state"),
        Err("get_state returned an MCP error".into())
    );
}
