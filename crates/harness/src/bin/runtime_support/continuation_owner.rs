// SPDX-License-Identifier: MIT

//! Authenticated current-owner read and durable selected-branch claim.

use std::collections::BTreeMap;

use serde_json::Value;
use sts2_harness::{BranchContinuationClaim, BranchContinuationClaimState, SqliteBranchStore};

use super::super::{config::RuntimeConfig, http::GatewayClient};
use super::allocation_context::RecoveryAuthority;
use wire::{
    claim_owner, ensure_current_owner, ensure_lookup_matches, lookup_claim, read_available_owner,
    validate_allocation_binding, validate_claim_response,
};

const CONTRACT: &str = "sts2-continuation-owner-v1";
const SCHEMA_DIGEST: &str = "5e787126c98cf950b94dcb4e02c5520ebbc5e0571a5e827cf0bd286815cd49dc";
const READ_PATH: &str = "/v1/recovery/continuation/owner/read";
const CLAIM_PATH: &str = "/v1/recovery/continuation/owner/claim";
const LOOKUP_PATH: &str = "/v1/recovery/continuation/owner/lookup";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[path = "continuation_owner_adopt.rs"]
mod adoption;
#[path = "continuation_owner_wire.rs"]
mod wire;

pub(crate) use adoption::adopt_current_owner;

/// Input that binds the selected branch's stable claim journal to the runtime allocation.
#[derive(Clone, Debug)]
pub(super) struct ContinuationOwnerClaimContext {
    pub(super) branch_store_path: std::path::PathBuf,
    pub(super) claim: BranchContinuationClaim,
}

/// Reads and claims the current allocated owner before MCP launch or any observation.
///
/// The gateway remains authoritative for lease liveness. This method persists the exact
/// non-secret owner tuple before the claim request and reconciles uncertain claim responses only
/// through a fresh current-owner read plus the same operation's historical lookup.
pub(super) fn claim_current_owner(
    config: &RuntimeConfig,
    recovery_authority: Option<&RecoveryAuthority>,
    context: &ContinuationOwnerClaimContext,
) -> Result<(), String> {
    let recovery_token = config
        .recovery_value("STS2_RECOVERY_TOKEN")
        .ok_or_else(|| String::from("continuation owner claim requires recovery credentials"))?;
    let recovery_principal = config
        .recovery_value("STS2_RECOVERY_PRINCIPAL_ID")
        .ok_or_else(|| String::from("continuation owner claim requires a recovery principal"))?;
    if recovery_principal != config.caller_id {
        return Err(String::from(
            "continuation owner recovery principal does not match the configured caller",
        ));
    }
    let client = GatewayClient::with_bearer(&config.gateway_address, recovery_token)?;
    let mut gateway = HttpOwnerGateway { client };
    claim_current_owner_with_gateway(config, recovery_authority, context, &mut gateway)
}

trait OwnerGatewayPort {
    fn post(&mut self, path: &str, capability: &str, frame: &Value) -> Result<Value, String>;
}

struct HttpOwnerGateway {
    client: GatewayClient,
}

impl OwnerGatewayPort for HttpOwnerGateway {
    fn post(&mut self, path: &str, capability: &str, frame: &Value) -> Result<Value, String> {
        let headers = BTreeMap::from([(
            String::from("x-sts2-recovery-capability"),
            capability.to_owned(),
        )]);
        self.client.request("POST", path, frame, headers)
    }
}

fn claim_current_owner_with_gateway<P: OwnerGatewayPort>(
    config: &RuntimeConfig,
    recovery_authority: Option<&RecoveryAuthority>,
    context: &ContinuationOwnerClaimContext,
    gateway: &mut P,
) -> Result<(), String> {
    let owner = read_available_owner(gateway, config)?;
    validate_allocation_binding(&owner, config, recovery_authority)?;

    let owner_json = serde_json::to_string(&owner)
        .map_err(|_| String::from("current owner could not be normalized"))?;
    let store = SqliteBranchStore::open(&context.branch_store_path)
        .map_err(|error| format!("cannot open branch owner-claim journal: {error}"))?;
    let claim = store
        .continuation_claim(&context.claim.experiment_id, &context.claim.branch_id)
        .map_err(|error| format!("cannot read branch owner-claim journal: {error}"))?
        .ok_or_else(|| String::from("selected branch owner-claim intent disappeared"))?;
    if claim.operation_id != context.claim.operation_id {
        return Err(String::from(
            "selected branch owner-claim operation identity changed",
        ));
    }
    match (claim.owner_json.as_deref(), claim.owner_digest.as_deref()) {
        (Some(owner_json), Some(owner_digest))
            if sts2_harness::sha256_hex(owner_json.as_bytes()) == owner_digest => {}
        (None, None) => {}
        _ => {
            return Err(String::from(
                "persisted selected-branch owner tuple failed its integrity check",
            ));
        }
    }
    let claim = match claim.owner_json.as_deref() {
        Some(existing) if existing == owner_json => claim,
        Some(_) => {
            mark_unknown_if_claimed(&store, &claim)?;
            return Err(String::from(
                "current gateway owner differs from the persisted selected-branch owner; continuation is refused",
            ));
        }
        None => store
            .snapshot_continuation_owner(&claim.operation_id, &owner_json)
            .map_err(|error| format!("cannot persist current gateway owner: {error}"))?,
    };

    if matches!(
        claim.state,
        BranchContinuationClaimState::Claimed | BranchContinuationClaimState::BoundaryVerified
    ) {
        let historic = lookup_claim(gateway, config, &claim.operation_id)?;
        ensure_lookup_matches(&historic, &claim.operation_id, &owner)?;
        return Ok(());
    }

    if claim.state == BranchContinuationClaimState::Unknown {
        let historic = lookup_claim(gateway, config, &claim.operation_id)?;
        if historic["payload"]["claim_state"] == "historical" {
            ensure_lookup_matches(&historic, &claim.operation_id, &owner)?;
            store
                .transition_continuation_claim(
                    &claim.operation_id,
                    BranchContinuationClaimState::Unknown,
                    BranchContinuationClaimState::Claimed,
                )
                .map_err(|error| {
                    format!("cannot reconcile selected-branch owner claim: {error}")
                })?;
            return Ok(());
        }
        ensure_current_owner(&historic, &owner)?;
    }

    if !matches!(
        claim.state,
        BranchContinuationClaimState::OwnerSnapshotted | BranchContinuationClaimState::Unknown
    ) {
        return Err(String::from(
            "selected branch owner-claim journal is not ready for gateway claim",
        ));
    }

    let prior = lookup_claim(gateway, config, &claim.operation_id)?;
    if prior["payload"]["claim_state"] == "historical" {
        ensure_lookup_matches(&prior, &claim.operation_id, &owner)?;
        store
            .transition_continuation_claim(
                &claim.operation_id,
                claim.state,
                BranchContinuationClaimState::Claimed,
            )
            .map_err(|error| format!("cannot reconcile selected-branch owner claim: {error}"))?;
        return Ok(());
    }
    ensure_current_owner(&prior, &owner)?;

    let expected_state = claim.state;
    let response = claim_owner(gateway, config, &claim.operation_id, &owner);
    match response {
        Ok(response) => {
            if let Err(error) =
                validate_claim_response(&response, config, &claim.operation_id, &owner)
            {
                mark_claim_unknown(&store, &claim.operation_id, expected_state)?;
                return Err(error);
            }
        }
        Err(_) => {
            let historic = lookup_claim(gateway, config, &claim.operation_id)?;
            if historic["payload"]["claim_state"] == "historical" {
                ensure_lookup_matches(&historic, &claim.operation_id, &owner)?;
            } else {
                ensure_current_owner(&historic, &owner)?;
                // The owner route is idempotent for this durable operation UUID. Retrying this
                // same claim cannot claim a second lease or repeat a game effect.
                match claim_owner(gateway, config, &claim.operation_id, &owner) {
                    Ok(response) => {
                        if let Err(error) =
                            validate_claim_response(&response, config, &claim.operation_id, &owner)
                        {
                            mark_claim_unknown(&store, &claim.operation_id, expected_state)?;
                            return Err(error);
                        }
                    }
                    Err(_) => {
                        if expected_state != BranchContinuationClaimState::Unknown {
                            store
                                .transition_continuation_claim(
                                    &claim.operation_id,
                                    expected_state,
                                    BranchContinuationClaimState::Unknown,
                                )
                                .map_err(|error| {
                                    format!("cannot record uncertain owner claim: {error}")
                                })?;
                        }
                        return Err(String::from(
                            "gateway owner claim remains uncertain; selected branch is non-executable",
                        ));
                    }
                }
            }
        }
    }
    store
        .transition_continuation_claim(
            &claim.operation_id,
            expected_state,
            BranchContinuationClaimState::Claimed,
        )
        .map_err(|error| format!("cannot persist gateway owner claim: {error}"))?;
    Ok(())
}

fn mark_claim_unknown(
    store: &SqliteBranchStore,
    operation_id: &str,
    expected_state: BranchContinuationClaimState,
) -> Result<(), String> {
    if expected_state != BranchContinuationClaimState::Unknown {
        store
            .transition_continuation_claim(
                operation_id,
                expected_state,
                BranchContinuationClaimState::Unknown,
            )
            .map_err(|error| format!("cannot record uncertain owner-claim response: {error}"))?;
    }
    Ok(())
}

fn mark_unknown_if_claimed(
    store: &SqliteBranchStore,
    claim: &BranchContinuationClaim,
) -> Result<(), String> {
    if matches!(
        claim.state,
        BranchContinuationClaimState::Claimed | BranchContinuationClaimState::OwnerSnapshotted
    ) {
        store
            .transition_continuation_claim(
                &claim.operation_id,
                claim.state,
                BranchContinuationClaimState::Unknown,
            )
            .map_err(|error| format!("cannot mark stale owner claim unknown: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "continuation_owner_tests.rs"]
mod tests;
