// SPDX-License-Identifier: MIT

//! Separate, closed-v1 route for adopting a selected branch's existing live allocation.

use serde_json::{Map, Value, json};
use sts2_harness::{BranchContinuationClaim, BranchContinuationClaimState, SqliteBranchStore};
use uuid::Uuid;

use super::super::super::config::RuntimeConfig;
use super::super::allocation_context::{self, ValidatedAllocation};
use super::{ContinuationOwnerClaimContext, HttpOwnerGateway, OwnerGatewayPort};

const CONTRACT: &str = "sts2-continuation-owner-adopt-v1";
const SCHEMA_DIGEST: &str = "7240e2f5054e5f6639ad12fa1ed66d40992f1234e49b693b2aacc53451dec380";
const CAPABILITY: &str = "continuation_owner_adopt";
const ADOPT_PATH: &str = "/v1/recovery/continuation/owner/adopt";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(crate) fn adopt_current_owner(
    config: &RuntimeConfig,
    context: &ContinuationOwnerClaimContext,
) -> Result<ValidatedAllocation, String> {
    let recovery_token = config
        .recovery_value("STS2_RECOVERY_TOKEN")
        .ok_or_else(|| String::from("selected branch resume requires recovery credentials"))?;
    let recovery_principal = config
        .recovery_value("STS2_RECOVERY_PRINCIPAL_ID")
        .ok_or_else(|| String::from("selected branch resume requires a recovery principal"))?;
    if recovery_principal != config.caller_id {
        return Err(String::from(
            "selected branch recovery principal does not match the configured caller",
        ));
    }
    let client = super::super::super::http::GatewayClient::with_bearer(
        &config.gateway_address,
        recovery_token,
    )?;
    let mut gateway = HttpOwnerGateway { client };
    adopt_current_owner_with_gateway(config, context, &mut gateway)
}

fn adopt_current_owner_with_gateway<P: OwnerGatewayPort>(
    config: &RuntimeConfig,
    context: &ContinuationOwnerClaimContext,
    gateway: &mut P,
) -> Result<ValidatedAllocation, String> {
    if config
        .recovery_value("STS2_RECOVERY_PRINCIPAL_ID")
        .is_none_or(|principal| principal != config.caller_id)
    {
        return Err(String::from(
            "selected branch recovery principal does not match the configured caller",
        ));
    }
    let claim = read_persisted_claim(context)?;
    let owner_json = claim
        .owner_json
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no persisted live owner fence"))?;
    let expected_owner: Value = super::super::super::gateway_json::parse(owner_json.as_bytes())
        .map_err(|_| String::from("persisted selected-branch owner fence is invalid JSON"))?;
    validate_owner(&expected_owner)?;
    if expected_owner["instance_id"] != config.instance_id
        || expected_owner["session_id"] != config.session_id
    {
        return Err(String::from(
            "persisted selected-branch owner does not match the configured instance and session",
        ));
    }
    let now = now_millis()?;
    if expected_owner["lease_expires_at_millis"]
        .as_u64()
        .is_none_or(|expiry| expiry <= now)
    {
        return Err(String::from(
            "persisted selected-branch owner lease is expired; resume is refused",
        ));
    }

    let correlation_id = Uuid::new_v4().to_string();
    let request = json!({
        "contract": CONTRACT,
        "schema_digest": SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": correlation_id,
        "actor": {"principal_id": config.caller_id, "role": "harness"},
        "auth": {
            "principal_id": config.caller_id,
            "capability": CAPABILITY,
            "proof": Value::Null
        },
        "kind": "owner_adopt_request",
        "payload": {
            "operation_id": claim.operation_id,
            "expected_owner": expected_owner
        }
    });
    let response = gateway.post(ADOPT_PATH, CAPABILITY, &request)?;
    validate_response(&response, config, &correlation_id, &claim, &expected_owner)
}

fn read_persisted_claim(
    context: &ContinuationOwnerClaimContext,
) -> Result<BranchContinuationClaim, String> {
    let store = SqliteBranchStore::open(&context.branch_store_path)
        .map_err(|error| format!("cannot open selected-branch resume journal: {error}"))?;
    let claim = store
        .continuation_claim(&context.claim.experiment_id, &context.claim.branch_id)
        .map_err(|error| format!("cannot read selected-branch resume journal: {error}"))?
        .ok_or_else(|| String::from("selected-branch resume claim disappeared"))?;
    if claim.operation_id != context.claim.operation_id {
        return Err(String::from(
            "selected-branch resume operation identity changed",
        ));
    }
    if claim.state != BranchContinuationClaimState::Resuming {
        return Err(String::from(
            "selected-branch owner claim is not held by an active resume attempt",
        ));
    }
    let owner_json = claim
        .owner_json
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no persisted live owner fence"))?;
    let owner_digest = claim
        .owner_digest
        .as_deref()
        .ok_or_else(|| String::from("selected branch owner fence has no digest"))?;
    if sts2_harness::sha256_hex(owner_json.as_bytes()) != owner_digest {
        return Err(String::from(
            "persisted selected-branch owner fence failed its digest check",
        ));
    }
    Ok(claim)
}

fn validate_response(
    frame: &Value,
    config: &RuntimeConfig,
    correlation_id: &str,
    claim: &BranchContinuationClaim,
    expected_owner: &Value,
) -> Result<ValidatedAllocation, String> {
    let now = now_millis()?;
    let object = exact_object(
        frame,
        &[
            "contract",
            "schema_digest",
            "message_id",
            "correlation_id",
            "actor",
            "auth",
            "kind",
            "payload",
        ],
        "adopt response",
    )?;
    let actor = exact_object(&frame["actor"], &["principal_id", "role"], "adopt actor")?;
    let auth = exact_object(
        &frame["auth"],
        &["principal_id", "capability", "proof"],
        "adopt auth",
    )?;
    let payload = exact_object(
        &frame["payload"],
        &["result", "claim", "owner", "recovery_authority"],
        "adopt payload",
    )?;
    let returned_claim = exact_object(
        &frame["payload"]["claim"],
        &[
            "operation_id",
            "request_digest",
            "owner",
            "claimed_at_millis",
        ],
        "adopt claim",
    )?;
    let _ = (object, actor, auth, payload, returned_claim);
    if frame["contract"] != CONTRACT
        || frame["schema_digest"] != SCHEMA_DIGEST
        || !uuid_v4(frame["message_id"].as_str())
        || frame["correlation_id"].as_str() != Some(correlation_id)
        || frame["kind"] != "owner_adopt_response"
        || frame["actor"]["principal_id"] != config.caller_id
        || frame["actor"]["role"] != "gateway"
        || frame["auth"]["principal_id"] != config.caller_id
        || frame["auth"]["capability"] != CAPABILITY
        || !frame["auth"]["proof"].is_null()
        || frame["payload"]["result"] != "ADOPTED"
    {
        return Err(String::from(
            "gateway selected-branch adopt response did not match its closed contract",
        ));
    }
    if frame["payload"]["claim"]["operation_id"] != claim.operation_id
        || !lower_sha256(frame["payload"]["claim"]["request_digest"].as_str())
        || positive_u53(&frame["payload"]["claim"]["claimed_at_millis"]).is_none()
        || frame["payload"]["claim"]["owner"] != *expected_owner
        || frame["payload"]["owner"] != *expected_owner
    {
        return Err(String::from(
            "gateway adopted a different selected-branch owner claim",
        ));
    }
    validate_owner(&frame["payload"]["owner"])?;
    if frame["payload"]["owner"]["lease_expires_at_millis"]
        .as_u64()
        .is_none_or(|expiry| expiry <= now)
    {
        return Err(String::from(
            "gateway returned an expired selected-branch owner lease",
        ));
    }
    let owner = &frame["payload"]["owner"];
    let authority = &frame["payload"]["recovery_authority"];
    let lease_id = owner["lease_id"]
        .as_str()
        .ok_or_else(|| String::from("adopted owner has no lease identity"))?;
    let lease_epoch = owner["lease_epoch"]
        .as_u64()
        .ok_or_else(|| String::from("adopted owner has no lease epoch"))?;
    let allocation = json!({
        "status": "allocated",
        "instance_id": config.instance_id,
        "caller_id": config.caller_id,
        "session_id": config.session_id,
        "lease_id": lease_id,
        "lease_epoch": lease_epoch,
        "recovery_authority": authority
    });
    let validated = allocation_context::validate(&allocation, config)?;
    if validated.lease_id != lease_id
        || validated.lease_epoch != lease_epoch
        || validated
            .recovery_authority
            .as_ref()
            .is_none_or(|authority| {
                authority.deployment_id != owner["deployment_id"]
                    || authority.instance_id != owner["instance_id"]
                    || authority.instance_incarnation != owner["instance_incarnation"]
                    || authority.boot_id != owner["boot_id"]
                    || authority.authority_generation != owner["authority_generation"]
                    || authority.lease_id != owner["lease_id"]
                    || authority.lease_epoch != owner["lease_epoch"]
                    || authority.current_fence["host_fence_id"] != owner["host_fence_id"]
                    || authority.current_fence["fence_generation"] != owner["host_fence_generation"]
            })
    {
        return Err(String::from(
            "gateway adopted recovery authority does not match the retained owner fence",
        ));
    }
    Ok(validated)
}

include!("continuation_owner_adopt_validation.rs");

#[cfg(test)]
#[path = "continuation_owner_adopt_tests.rs"]
mod tests;
