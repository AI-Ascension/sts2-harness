// SPDX-License-Identifier: MIT

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::durable::DurableHandle;
use sha2::Digest;
use sts2_harness::{
    ActionIdentity, ActionKind, Decision, DecisionInput, DecisionSource, EpisodeLegalAction,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ModelExecutionId, PolicyError,
    RecoveryPort,
};

const PENDING_OPERATION_ID: &str = "11111111-1111-4111-8111-111111111111";
const PENDING_STATE_ID: &str = "22222222-2222-4222-8222-222222222222";
const RECOVERY_DEPLOYMENT_ID: &str = "33333333-3333-4333-8333-333333333333";
const RECOVERY_INSTANCE_ID: &str = "44444444-4444-4444-8444-444444444444";
const RECOVERY_INSTANCE_INCAR: &str = "55555555-5555-4555-8555-555555555555";
const RECOVERY_BOOT_ID: &str = "66666666-6666-4666-8666-666666666666";
const RECOVERY_LEASE_ID: &str = "77777777-7777-4777-8777-777777777777";
const RECOVERY_FENCE_ID: &str = "88888888-8888-4888-8888-888888888888";

struct RecoveryEnvironment(Vec<(String, String)>);

impl RecoveryEnvironment {
    fn new() -> Self {
        let fence = json!({
            "host_fence_id": RECOVERY_FENCE_ID,
            "deployment_id": RECOVERY_DEPLOYMENT_ID,
            "instance_id": RECOVERY_INSTANCE_ID,
            "instance_incarnation": RECOVERY_INSTANCE_INCAR,
            "boot_id": RECOVERY_BOOT_ID,
            "authority_generation": 1,
            "fence_generation": 1,
            "created_at": "2026-09-07T00:00:00Z"
        });
        Self(
            vec![
                ("STS2_RECOVERY_TOKEN", "synthetic-recovery-token".to_owned()),
                (
                    "STS2_RECOVERY_PRINCIPAL_ID",
                    "synthetic-principal".to_owned(),
                ),
                ("STS2_RECOVERY_ROLE", "watchdog-recovery".to_owned()),
                ("STS2_RECOVERY_PROOF", "synthetic-proof".to_owned()),
                (
                    "STS2_RECOVERY_DEPLOYMENT_ID",
                    RECOVERY_DEPLOYMENT_ID.to_owned(),
                ),
                ("STS2_RECOVERY_INSTANCE_ID", RECOVERY_INSTANCE_ID.to_owned()),
                (
                    "STS2_RECOVERY_INSTANCE_INCAR",
                    RECOVERY_INSTANCE_INCAR.to_owned(),
                ),
                ("STS2_RECOVERY_BOOT_ID", RECOVERY_BOOT_ID.to_owned()),
                ("STS2_RECOVERY_LEASE_ID", RECOVERY_LEASE_ID.to_owned()),
                ("STS2_RECOVERY_AUTHORITY_GENERATION", String::from("1")),
                ("STS2_RECOVERY_LEASE_EPOCH", String::from("1")),
                ("STS2_RECOVERY_CURRENT_FENCE_JSON", fence.to_string()),
            ]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
        )
    }
}

use super::*;

struct Fixture(PathBuf);

struct CountingSource {
    calls: usize,
    decision: Decision,
}

impl DecisionSource for CountingSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.calls += 1;
        Ok(self.decision.clone())
    }
}

impl Fixture {
    fn new() -> Result<Self, std::io::Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sts2-v3-reconnect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn script(&self, content: &str) -> Result<String, Box<dyn std::error::Error>> {
        let path = self.0.join("mcp");
        fs::write(&path, format!("#!/bin/sh\n{content}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(path.to_str().ok_or("non-UTF8 fixture path")?.to_owned())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn reply(value: Value) -> String {
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
        value.to_string().replace('\'', "'\\''")
    )
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[((first & 0x03) << 4 | second.unwrap_or(0) >> 4) as usize] as char);
        encoded.push(match second {
            Some(second) => {
                TABLE[((second & 0x0f) << 2 | third.unwrap_or(0) >> 6) as usize] as char
            }
            None => '=',
        });
        encoded.push(match third {
            Some(third) => TABLE[(third & 0x3f) as usize] as char,
            None => '=',
        });
    }
    encoded
}

fn dispatch_script(fixture: &Fixture) -> Result<String, Box<dyn std::error::Error>> {
    let mut settled: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))?;
    settled["correlation_id"] = json!("1");
    settled["operation_id"] = json!("op-settled");
    let tools: Vec<_> = [
        "sts2.observe",
        "sts2.legal_actions",
        "sts2.dispatch_action",
        "sts2.wait_for_transition",
        "sts2.reobserve",
        "sts2.recover",
    ]
    .into_iter()
    .map(|name| json!({"name":name}))
    .collect();
    let script = format!(
        "cd '{}' || exit 1\n{}{}{}",
        fixture.0.display(),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":tools}})
        ),
        reply(json!({
            "jsonrpc":"2.0",
            "id":1,
            "result":{"content":[{"text":settled.to_string()}]}
        }))
    );
    fixture.script(&script)
}

fn recovery_settled_script(
    fixture: &Fixture,
    operation_id: &str,
    state_id: &str,
    generation: u64,
    payload_digest: &str,
    catalog_digest: &str,
    canonical_json_b64: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut observed: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    observed["correlation_id"] = json!("1");
    observed["generation"] = json!(1);
    observed["observation"]["generation"] = json!(1);
    observed["observation"]["state"]["turn_index"] = json!(2);
    let gameplay_tools: Vec<_> = [
        "sts2.observe",
        "sts2.legal_actions",
        "sts2.dispatch_action",
        "sts2.wait_for_transition",
        "sts2.reobserve",
        "sts2.recover",
    ]
    .into_iter()
    .map(|name| json!({"name":name}))
    .collect();
    let recovery_tools: Vec<_> = [
        "watchdog.bootstrap",
        "watchdog.host_fence",
        "watchdog.lease_acquire",
        "watchdog.lease_renew",
        "watchdog.lease_revoke",
        "watchdog.operation_intent",
        "watchdog.operation_dispatch",
        "watchdog.operation_lookup",
        "watchdog.operation_reconcile",
    ]
    .into_iter()
    .map(|name| json!({"name":name}))
    .collect();
    let operation = |state: &str| {
        json!({
            "operation_id": operation_id,
            "payload_digest": payload_digest,
            "expected_boundary": {
                "state_id": state_id,
                "generation": generation,
                "catalog_digest": catalog_digest
            },
            "action": {
                "schema_digest": wire::RUNTIME_V3_SCHEMA_DIGEST,
                "canonical_json_b64": canonical_json_b64,
                "payload_digest": payload_digest
            },
            "state": state
        })
    };
    let lookup = json!({
        "contract": "watchdog-recovery-v1",
        "schema_digest": sts2_harness::RECOVERY_SCHEMA_DIGEST,
        "correlation_id": "1",
        "kind": "operation_lookup_response",
        "payload": {
            "operation": operation("SETTLED"),
            "mutation_authorized": false,
            "result": {"status": "SETTLED"}
        }
    });
    let reconcile = json!({
        "contract": "watchdog-recovery-v1",
        "schema_digest": sts2_harness::RECOVERY_SCHEMA_DIGEST,
        "correlation_id": "2",
        "kind": "operation_reconcile_response",
        "payload": {
            "operation": operation("RECONCILED"),
            "mutation_authorized": false,
            "result": {"status": "RECONCILED"}
        }
    });
    let script = format!(
        "cd '{}' || exit 1\nif [ \"$STS2_RUNTIME_PROFILE\" = \"watchdog-recovery-v1\" ]; then\n{}{}{}{}else\n{}{}{}\nfi\n",
        fixture.0.display(),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"watchdog-recovery-v1-mcp","tools":recovery_tools}})
        ),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"text":lookup.to_string()}]}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"content":[{"text":reconcile.to_string()}]}})
        ),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":gameplay_tools}})
        ),
        reply(json!({
            "jsonrpc":"2.0",
            "id":1,
            "result":{"content":[{"text":observed.to_string()}]}
        }))
    );
    fixture.script(&script)
}

fn synthetic_observation(
    state_id: &str,
    generation: u64,
    stage: &str,
    legal_actions: Value,
) -> Result<sts2_harness::EpisodeObservation, Box<dyn std::error::Error>> {
    let state = if stage == "victory" {
        json!({"state": stage})
    } else {
        json!({"state": stage, "turn_index": 1, "enemies": []})
    };
    let observation = json!({
        "state_id": state_id,
        "generation": generation,
        "visible_seed": "synthetic-seed",
        "player": {"hp":50,"max_hp":50,"energy":3,"gold":99,"hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state": state,
        "legal_actions": legal_actions
    });
    Ok(sts2_harness::EpisodeObservation::new(
        state_id,
        generation,
        match stage {
            "combat" => sts2_harness::EpisodeStage::Combat,
            "victory" => sts2_harness::EpisodeStage::Victory,
            _ => return Err("unsupported synthetic stage".into()),
        },
        stage == "combat",
        stage != "combat",
        stage == "combat",
        observation,
    )?)
}

#[test]
fn durable_runtime_lifecycle_checkpoints_accounts_provider_and_reconciles_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let mut runtime_config = config("127.0.0.1:15525".into());
    runtime_config.mcp_binary = dispatch_script(&fixture)?;
    let lineage = ExecutionLineage::new(
        runtime_config.run_id.clone(),
        runtime_config.episode_id.clone(),
        "attempt-synthetic",
        runtime_config.trajectory_id.clone(),
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "synthetic-seed",
        "synthetic-build",
        "synthetic-state",
        "synthetic-config",
        "synthetic-provider",
    )?;
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable = DurableHandle::from_store_for_test(store, lineage.clone(), fingerprint)?;
    let mut port = RuntimeV3Port::new_with_store(
        runtime_config,
        TelemetryHandle::disabled(),
        durable.clone(),
    )?;
    port.allocated = true;
    let mut mcp = McpProcess::spawn(&port.config)?;
    wire::initialize_mcp(&mut mcp)?;
    port.mcp = Some(mcp);
    let mut state: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    state["legal_actions"] = json!([
        {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
    ]);
    let parsed = parse::observation(&state, "state_response", &port.config)?;
    let action = parsed.actions.actions()[0].clone();
    let observation = port.install(parsed)?;
    durable.refresh_resume_boundary()?;
    let divergent = synthetic_observation(
        observation.state_id(),
        observation.generation(),
        "combat",
        json!([{"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}]),
    )?;
    assert!(durable.verify_resume_boundary(&divergent).is_err());
    durable.verify_resume_boundary(&observation)?;
    let identity = ActionIdentity::new(
        "op-settled",
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    let receipt = port.dispatch_action(&identity, &action)?;
    assert_eq!(receipt.status(), sts2_harness::DispatchStatus::Settled);
    assert_eq!(
        durable.operation_state("op-settled")?,
        sts2_harness::OperationState::Settled
    );

    let input = DecisionInput::new(
        ModelExecutionId::new(1).ok_or("execution identity")?,
        observation.clone(),
        port.current_actions.clone().ok_or("missing catalog")?,
        "synthetic lifecycle",
        Vec::new(),
    );
    let reservation = match durable.decision_admission_with_reuse(&input)? {
        super::super::DecisionAdmission::Fresh(reservation) => reservation,
        super::super::DecisionAdmission::Reused(_) => {
            return Err("new decision unexpectedly reused a result".into());
        }
    };
    let decision = Decision::Action {
        action_id: action.action_id().to_owned(),
        rationale: String::from("synthetic provider decision"),
        confidence: Some(90),
    };
    durable.complete_decision(&reservation, &decision)?;
    let mut source = CountingSource {
        calls: 0,
        decision: Decision::Wait {
            rationale: String::from("provider must not be called"),
        },
    };
    let mut recorder = super::super::recording::DecisionRecorder::new(
        &mut source,
        TelemetryHandle::disabled(),
        Some(durable.clone()),
    );
    assert_eq!(recorder.decide(&input)?, decision);
    drop(recorder);
    assert_eq!(source.calls, 0);
    assert!(matches!(
        durable.decision_admission_with_reuse(&input)?,
        super::super::DecisionAdmission::Reused(_)
    ));

    let pending_action = EpisodeLegalAction::new("combat.end-turn-pending", ActionKind::EndTurn)?;
    let pending_catalog = json!([{
        "action_id": "combat.end-turn-pending",
        "action": {"kind": "end_turn"}
    }]);
    durable.operation_intent(
        PENDING_OPERATION_ID,
        PENDING_STATE_ID,
        observation.generation(),
        &pending_action,
        &json!({"kind":"end_turn"}),
        &json!({
            "state_id": PENDING_STATE_ID,
            "generation": observation.generation(),
            "legal_actions": pending_catalog
        }),
    )?;
    let pending_digest = durable.operation_payload_digest(PENDING_OPERATION_ID)?;
    let canonical_action =
        wire::canonical_action_bytes(pending_action.action_id(), &json!({"kind":"end_turn"}))?;
    let canonical_json_b64 = encode_base64(&canonical_action);
    let catalog_digest = format!(
        "{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&pending_catalog)?)
    );
    durable.operation_dispatched(PENDING_OPERATION_ID, &pending_digest)?;
    drop(port);

    let recovery_environment = RecoveryEnvironment::new();
    let mut resumed_config = config("127.0.0.1:15525".into());
    resumed_config.recovery_environment = recovery_environment.0;
    resumed_config.mcp_binary = recovery_settled_script(
        &fixture,
        PENDING_OPERATION_ID,
        PENDING_STATE_ID,
        observation.generation(),
        &pending_digest,
        &catalog_digest,
        &canonical_json_b64,
    )?;
    let mut resumed = RuntimeV3Port::new_with_store(
        resumed_config,
        TelemetryHandle::disabled(),
        durable.clone(),
    )?;
    resumed.allocated = true;
    let mut mcp = McpProcess::spawn(&resumed.config)?;
    wire::initialize_mcp(&mut mcp)?;
    resumed.mcp = Some(mcp);
    resumed.reconcile_pending_operations()?;
    assert_eq!(
        durable.operation_state(PENDING_OPERATION_ID)?,
        sts2_harness::OperationState::Reconciled
    );
    let resumed_observation = resumed.observe()?;
    assert_eq!(resumed_observation.generation(), 1);

    let terminal = synthetic_observation("victory-2", 2, "victory", json!([]))?;
    durable.checkpoint(&terminal, &json!({}))?;
    resumed.complete_durable_observation(&terminal)?;
    assert!(durable.decision_admission_with_reuse(&input).is_err());
    resumed.mcp.as_mut().ok_or("missing MCP")?.close()?;
    Ok(())
}

#[test]
fn runtime_v3_reconnect_requires_durable_identity_without_redispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let mut config = config("127.0.0.1:15525".into());
    config.mcp_binary = fixture.script(&format!(
        "cd '{}' || exit 1\nIFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0\n",
        fixture.0.display()
    ))?;
    let mut port = RuntimeV3Port::new_with_telemetry(config, TelemetryHandle::disabled())?;
    port.allocated = true;
    port.mcp = Some(McpProcess::spawn(&port.config)?);
    let mut state: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    state["legal_actions"] = json!([
        {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
    ]);
    let parsed = parse::observation(&state, "state_response", &port.config)?;
    let action = parsed.actions.actions()[0].clone();
    let observation = port.install(parsed)?;
    let identity = ActionIdentity::new(
        "op-1",
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    assert!(port.dispatch_action(&identity, &action).is_err());
    assert!(port.mcp.as_ref().is_some_and(McpProcess::is_closed));
    assert!(port.reconcile("op-1").is_err());
    assert_eq!(port.operations.len(), 1);
    let requests = fs::read_to_string(fixture.0.join("requests"))?;
    let requests: Vec<Value> = requests
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["params"]["name"], "sts2.dispatch_action");
    assert!(
        !requests
            .iter()
            .any(|value| value["params"]["name"] == "sts2.recover")
    );
    Ok(())
}
