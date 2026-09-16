// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::path::PathBuf;

use serde_json::{Value, json};
use sts2_harness::{
    BranchAssurance, BranchFork, BranchStrategy, DurableBranchDraft, ExactStateDigest,
    OccurrenceId, SqliteBranchStore,
};
use uuid::Uuid;

use super::super::allocation_context::RecoveryAuthority;
use super::{
    CLAIM_PATH, ContinuationOwnerClaimContext, LOOKUP_PATH, OwnerGatewayPort, READ_PATH,
    claim_current_owner_with_gateway,
};
use crate::runtime_support::config::RuntimeConfig;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "sts2-continuation-owner-test-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct RecordingOwnerGateway {
    owner: Value,
    state: &'static str,
    claim_seen: bool,
    lose_first_claim_ack: bool,
    fail_first_claim_before_commit: bool,
    calls: Vec<(String, String, Value)>,
}

impl OwnerGatewayPort for RecordingOwnerGateway {
    fn post(&mut self, path: &str, capability: &str, frame: &Value) -> Result<Value, String> {
        self.calls
            .push((path.to_owned(), capability.to_owned(), frame.clone()));
        if frame["contract"] != "sts2-continuation-owner-v1"
            || frame["schema_digest"]
                != "5e787126c98cf950b94dcb4e02c5520ebbc5e0571a5e827cf0bd286815cd49dc"
            || frame["actor"]["role"] != "harness"
            || frame["actor"]["principal_id"] != "harness"
            || frame["auth"]["principal_id"] != "harness"
            || !frame["auth"]["proof"].is_null()
        {
            return Err(String::from(
                "owner request frame failed test-boundary checks",
            ));
        }
        match path {
            READ_PATH => {
                if capability != "continuation_owner_read"
                    || frame["kind"] != "current_owner_request"
                {
                    return Err(String::from(
                        "owner read route capability or kind was invalid",
                    ));
                }
                Ok(response(
                    frame,
                    "current_owner_response",
                    json!({"state": self.state, "owner": self.owner}),
                ))
            }
            CLAIM_PATH => {
                if capability != "continuation_owner_claim"
                    || frame["kind"] != "owner_claim_request"
                {
                    return Err(String::from(
                        "owner claim route capability or kind was invalid",
                    ));
                }
                if frame["payload"]["expected_owner"] != self.owner {
                    return Err(String::from(
                        "claim request owner did not match current owner",
                    ));
                }
                if self.fail_first_claim_before_commit {
                    self.fail_first_claim_before_commit = false;
                    return Err(String::from("simulated claim failure before commit"));
                }
                self.claim_seen = true;
                if self.lose_first_claim_ack {
                    self.lose_first_claim_ack = false;
                    return Err(String::from("simulated lost owner-claim acknowledgement"));
                }
                Ok(response(
                    frame,
                    "owner_claim_response",
                    json!({
                        "result":"CLAIMED",
                        "claim":{
                            "operation_id":frame["payload"]["operation_id"],
                            "request_digest":"a".repeat(64),
                            "owner":self.owner,
                            "claimed_at_millis":1_800_000_000_000_u64
                        }
                    }),
                ))
            }
            LOOKUP_PATH => {
                if capability != "continuation_owner_lookup"
                    || frame["kind"] != "owner_claim_lookup_request"
                {
                    return Err(String::from(
                        "owner lookup route capability or kind was invalid",
                    ));
                }
                let (claim_state, claim) = if self.claim_seen {
                    (
                        "historical",
                        json!({
                            "operation_id":frame["payload"]["operation_id"],
                            "request_digest":"a".repeat(64),
                            "owner":self.owner,
                            "claimed_at_millis":1_800_000_000_000_u64
                        }),
                    )
                } else {
                    ("not_found", Value::Null)
                };
                Ok(response(
                    frame,
                    "owner_claim_lookup_response",
                    json!({
                        "claim_state":claim_state,
                        "claim":claim,
                        "current_owner":{"state":self.state,"owner":self.owner}
                    }),
                ))
            }
            _ => Err(String::from("unexpected continuation-owner route")),
        }
    }
}

fn response(request: &Value, kind: &str, payload: Value) -> Value {
    json!({
        "contract":"sts2-continuation-owner-v1",
        "schema_digest":"5e787126c98cf950b94dcb4e02c5520ebbc5e0571a5e827cf0bd286815cd49dc",
        "message_id":Uuid::new_v4().to_string(),
        "correlation_id":request["correlation_id"],
        "actor":{"principal_id":"harness","role":"gateway"},
        "auth":{
            "principal_id":"harness",
            "capability":request["auth"]["capability"],
            "proof":Value::Null
        },
        "kind":kind,
        "payload":payload
    })
}

fn setup() -> Result<
    (
        Scratch,
        RuntimeConfig,
        RecoveryAuthority,
        ContinuationOwnerClaimContext,
    ),
    String,
> {
    const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
    const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
    const INCARNATION: &str = "00000000-0000-4000-8000-000000000003";
    const BOOT: &str = "00000000-0000-4000-8000-000000000004";
    const FENCE: &str = "00000000-0000-4000-8000-000000000005";
    const LEASE: &str = "00000000-0000-4000-8000-000000000006";
    let scratch = Scratch::new()?;
    let branch_path = scratch.0.join("branches.sqlite3");
    let store = SqliteBranchStore::open(&branch_path).map_err(|error| error.to_string())?;
    let digest = ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "b".repeat(64)))
        .map_err(|error| error.to_string())?;
    store
        .create(
            "create-root",
            DurableBranchDraft {
                experiment_id: "experiment:owner-test".to_owned(),
                root_branch_id: "branch:root".to_owned(),
                branch_id: "branch:root".to_owned(),
                parent_branch_id: None,
                fork: BranchFork {
                    occurrence_id: OccurrenceId::parse("occurrence:root")
                        .map_err(|error| format!("{error:?}"))?,
                    parent_occurrence_id: None,
                    state_digest: digest.clone(),
                },
                strategy: BranchStrategy::ExactRestore,
                source_handle: Some(String::from("checkpoint:source")),
                trajectory_prefix: None,
                effective_seed: None,
                setup_digest: None,
                boundary: String::from("decision"),
                assurance: BranchAssurance::Unverified,
                run_id: String::from("run:root"),
                episode_id: Some(String::from("episode:root")),
                trajectory_id: Some(String::from("trajectory:root")),
                context_id: Some(String::from("context:root")),
                policy_revision: String::from("policy:test"),
                config_revision: String::from("config:test"),
                name: String::from("root"),
                notes: None,
                artifacts: Vec::new(),
            },
        )
        .map_err(|error| error.to_string())?;
    store
        .create(
            "create-selected",
            DurableBranchDraft {
                experiment_id: "experiment:owner-test".to_owned(),
                root_branch_id: "branch:root".to_owned(),
                branch_id: "branch:selected".to_owned(),
                parent_branch_id: Some(String::from("branch:root")),
                fork: BranchFork {
                    occurrence_id: OccurrenceId::parse("occurrence:selected")
                        .map_err(|error| format!("{error:?}"))?,
                    parent_occurrence_id: Some(
                        OccurrenceId::parse("occurrence:root")
                            .map_err(|error| format!("{error:?}"))?,
                    ),
                    state_digest: digest,
                },
                strategy: BranchStrategy::PrefixReplay,
                source_handle: None,
                trajectory_prefix: Some(String::from("trajectory:prefix")),
                effective_seed: Some(String::from("seed:branch")),
                setup_digest: Some(String::from("setup:branch")),
                boundary: String::from("decision"),
                assurance: BranchAssurance::Unverified,
                run_id: String::from("run:selected"),
                episode_id: Some(String::from("episode:selected")),
                trajectory_id: Some(String::from("trajectory:selected")),
                context_id: Some(String::from("context:selected")),
                policy_revision: String::from("policy:test"),
                config_revision: String::from("config:test"),
                name: String::from("selected"),
                notes: None,
                artifacts: Vec::new(),
            },
        )
        .map_err(|error| error.to_string())?;
    let claim = store
        .prepare_continuation_claim("experiment:owner-test", "branch:selected")
        .map_err(|error| error.to_string())?;

    let config = RuntimeConfig {
        seed_transport: None,
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("normal-token"),
        mcp_binary: String::from("mcp"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: INSTANCE.to_owned(),
        caller_id: String::from("harness"),
        session_id: String::from("gateway-session"),
        lease_id: LEASE.to_owned(),
        lease_epoch: 8,
        mcp_session_id: String::from("mcp-session"),
        run_id: String::from("run:selected"),
        episode_id: String::from("episode:selected"),
        trajectory_id: String::from("trajectory:selected"),
        trace_id: String::from("trace:selected"),
        artifact_id: String::from("artifact:selected"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: vec![
            (
                String::from("STS2_RECOVERY_TOKEN"),
                String::from("recovery"),
            ),
            (
                String::from("STS2_RECOVERY_PRINCIPAL_ID"),
                String::from("harness"),
            ),
        ],
    };
    let authority = RecoveryAuthority {
        deployment_id: DEPLOYMENT.to_owned(),
        instance_id: INSTANCE.to_owned(),
        instance_incarnation: INCARNATION.to_owned(),
        boot_id: BOOT.to_owned(),
        authority_generation: 7,
        lease_id: LEASE.to_owned(),
        lease_epoch: 8,
        current_fence: json!({"host_fence_id":FENCE,"fence_generation":3}),
    };
    Ok((
        scratch,
        config,
        authority,
        ContinuationOwnerClaimContext {
            branch_store_path: branch_path,
            claim,
        },
    ))
}

fn owner_for(config: &RuntimeConfig, authority: &RecoveryAuthority) -> Value {
    json!({
        "deployment_id":authority.deployment_id,
        "instance_id":config.instance_id,
        "instance_incarnation":authority.instance_incarnation,
        "boot_id":authority.boot_id,
        "authority_generation":authority.authority_generation,
        "host_fence_id":authority.current_fence["host_fence_id"],
        "host_fence_generation":authority.current_fence["fence_generation"],
        "lease_id":config.lease_id,
        "lease_epoch":config.lease_epoch,
        "session_id":config.session_id,
        "lease_expires_at_millis":super::wire::now_millis().unwrap_or_default().saturating_add(3_600_000)
    })
}

#[test]
fn production_owner_gate_persists_read_then_claims_before_continuation() -> Result<(), String> {
    let (_scratch, config, authority, context) = setup()?;
    let owner = owner_for(&config, &authority);
    let mut gateway = RecordingOwnerGateway {
        owner,
        state: "available",
        claim_seen: false,
        lose_first_claim_ack: false,
        fail_first_claim_before_commit: false,
        calls: Vec::new(),
    };

    claim_current_owner_with_gateway(&config, Some(&authority), &context, &mut gateway)?;

    assert_eq!(
        gateway
            .calls
            .iter()
            .map(|(path, _, _)| path.as_str())
            .collect::<Vec<_>>(),
        [READ_PATH, LOOKUP_PATH, CLAIM_PATH]
    );
    assert_eq!(
        gateway
            .calls
            .iter()
            .map(|(_, capability, _)| capability.as_str())
            .collect::<Vec<_>>(),
        [
            "continuation_owner_read",
            "continuation_owner_lookup",
            "continuation_owner_claim"
        ]
    );
    let store =
        SqliteBranchStore::open(&context.branch_store_path).map_err(|error| error.to_string())?;
    let persisted = store
        .continuation_claim(&context.claim.experiment_id, &context.claim.branch_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("owner claim journal is missing"))?;
    assert_eq!(persisted.operation_id, context.claim.operation_id);
    assert_eq!(
        persisted.state,
        sts2_harness::BranchContinuationClaimState::Claimed
    );
    assert!(persisted.owner_digest.is_some());
    Ok(())
}

include!("continuation_owner_recovery_tests.rs");
