// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sts2_harness::{BranchContinuationClaim, BranchContinuationClaimState};
use uuid::Uuid;

use super::*;

const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
const INCARNATION: &str = "00000000-0000-4000-8000-000000000003";
const BOOT: &str = "00000000-0000-4000-8000-000000000004";
const FENCE: &str = "00000000-0000-4000-8000-000000000005";
const LEASE: &str = "00000000-0000-4000-8000-000000000006";
const PRINCIPAL: &str = "selected-principal";
const SESSION: &str = "selected-session";
const CORRELATION: &str = "00000000-0000-4000-8000-000000000020";

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        seed_transport: None,
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-gateway-token"),
        mcp_binary: String::from("sts2-mcp-server"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: INSTANCE.to_owned(),
        caller_id: PRINCIPAL.to_owned(),
        session_id: SESSION.to_owned(),
        lease_id: String::from("configured-stale-lease"),
        lease_epoch: 1,
        episode_profile: false,
        mcp_session_id: String::from("selected-mcp-session"),
        run_id: String::from("selected-run"),
        episode_id: String::from("selected-episode"),
        trajectory_id: String::from("selected-trajectory"),
        trace_id: String::from("selected-trace"),
        artifact_id: String::from("selected-artifact"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: vec![
            (
                String::from("STS2_RECOVERY_TOKEN"),
                String::from("synthetic-token"),
            ),
            (
                String::from("STS2_RECOVERY_PRINCIPAL_ID"),
                PRINCIPAL.to_owned(),
            ),
        ],
    }
}

fn owner(expiry: u64) -> Value {
    json!({
        "deployment_id": DEPLOYMENT,
        "instance_id": INSTANCE,
        "instance_incarnation": INCARNATION,
        "boot_id": BOOT,
        "authority_generation": 7,
        "host_fence_id": FENCE,
        "host_fence_generation": 3,
        "lease_id": LEASE,
        "lease_epoch": 8,
        "session_id": SESSION,
        "lease_expires_at_millis": expiry
    })
}

fn claim(operation_id: String, owner: &Value) -> BranchContinuationClaim {
    let owner_json = serde_json::to_string(owner).expect("owner encodes");
    BranchContinuationClaim {
        experiment_id: String::from("experiment:resume"),
        branch_id: String::from("branch:selected"),
        operation_id,
        state: BranchContinuationClaimState::Resuming,
        owner_digest: Some(sts2_harness::sha256_hex(owner_json.as_bytes())),
        owner_json: Some(owner_json),
    }
}

fn authority() -> Value {
    json!({
        "contract":"watchdog-runtime-allocation-v1",
        "schema_digest":"ee967a95e79fb2f157ce58d2b6d857de42b75f1f5ebfeb82dd9672e3b0f7670b",
        "context":{
            "deployment_id":DEPLOYMENT,
            "instance_id":INSTANCE,
            "instance_incarnation":INCARNATION,
            "boot_id":BOOT,
            "authority_generation":7,
            "lease_id":LEASE,
            "lease_epoch":8
        },
        "current_fence":{
            "host_fence_id":FENCE,
            "deployment_id":DEPLOYMENT,
            "instance_id":INSTANCE,
            "instance_incarnation":INCARNATION,
            "boot_id":BOOT,
            "authority_generation":7,
            "fence_generation":3,
            "created_at":"2026-09-16T00:00:00Z"
        }
    })
}

fn response(operation_id: &str, expected_owner: &Value, expiry: u64) -> Value {
    json!({
        "contract": CONTRACT,
        "schema_digest": SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": CORRELATION,
        "actor":{"principal_id":PRINCIPAL,"role":"gateway"},
        "auth":{"principal_id":PRINCIPAL,"capability":CAPABILITY,"proof":null},
        "kind":"owner_adopt_response",
        "payload":{
            "result":"ADOPTED",
            "claim":{
                "operation_id":operation_id,
                "request_digest":"a".repeat(64),
                "owner":expected_owner,
                "claimed_at_millis":expiry.saturating_sub(60_000)
            },
            "owner":expected_owner,
            "recovery_authority":authority()
        }
    })
}

#[test]
fn adopt_response_validates_exact_owner_and_returns_current_lease_authority() -> Result<(), String>
{
    let config = runtime_config();
    let expiry = now_millis()?.saturating_add(60_000);
    let expected_owner = owner(expiry);
    let operation_id = Uuid::new_v4().to_string();
    let selected_claim = claim(operation_id.clone(), &expected_owner);

    let allocation = validate_response(
        &response(&operation_id, &expected_owner, expiry),
        &config,
        CORRELATION,
        &selected_claim,
        &expected_owner,
    )?;

    assert_eq!(allocation.lease_id, LEASE);
    assert_eq!(allocation.lease_epoch, 8);
    let mut rebound = config;
    allocation.apply_current_lease(&mut rebound);
    assert_eq!(rebound.lease_id, LEASE);
    assert_eq!(rebound.lease_epoch, 8);
    Ok(())
}

#[test]
fn adopt_response_rejects_mismatched_claim_fence_or_extra_fields() -> Result<(), String> {
    let config = runtime_config();
    let expiry = now_millis()?.saturating_add(60_000);
    let expected_owner = owner(expiry);
    let operation_id = Uuid::new_v4().to_string();
    let selected_claim = claim(operation_id.clone(), &expected_owner);

    let mut mismatched_claim = response(&operation_id, &expected_owner, expiry);
    mismatched_claim["payload"]["claim"]["operation_id"] = json!(Uuid::new_v4().to_string());
    assert!(
        validate_response(
            &mismatched_claim,
            &config,
            CORRELATION,
            &selected_claim,
            &expected_owner,
        )
        .is_err()
    );

    let mut mismatched_fence = response(&operation_id, &expected_owner, expiry);
    mismatched_fence["payload"]["recovery_authority"]["current_fence"]["fence_generation"] =
        json!(4);
    assert!(
        validate_response(
            &mismatched_fence,
            &config,
            CORRELATION,
            &selected_claim,
            &expected_owner,
        )
        .is_err()
    );

    let mut extra_field = response(&operation_id, &expected_owner, expiry);
    extra_field["payload"]["unexpected"] = json!(true);
    assert!(
        validate_response(
            &extra_field,
            &config,
            CORRELATION,
            &selected_claim,
            &expected_owner,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn adopt_response_rejects_expired_owner_before_rebinding() -> Result<(), String> {
    let config = runtime_config();
    let expiry = now_millis()?.saturating_sub(1);
    let expected_owner = owner(expiry);
    let operation_id = Uuid::new_v4().to_string();
    let selected_claim = claim(operation_id.clone(), &expected_owner);

    assert!(
        validate_response(
            &response(&operation_id, &expected_owner, expiry),
            &config,
            CORRELATION,
            &selected_claim,
            &expected_owner,
        )
        .is_err()
    );
    Ok(())
}
