// SPDX-License-Identifier: MIT

use sts2_harness::{
    ActionKind, EpisodeLegalAction, ExecutionFingerprint, ExecutionLineage, ExecutionStore,
    OperationState,
};

use super::super::durable::DurableHandle;
use super::reconnect_support::*;
use super::*;

#[path = "runtime_v3_recovery_diagnostics_test.rs"]
mod diagnostics;

struct RecoveryCase {
    fixture: Fixture,
    durable: DurableHandle,
    lookup: Value,
    reconcile: Value,
}

impl RecoveryCase {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let fixture = Fixture::new()?;
        let lineage = ExecutionLineage::new("run-1", "episode-1", "attempt-1", "trajectory-1")?;
        let fingerprint =
            ExecutionFingerprint::new("seed", "build", "state", "config", "provider")?;
        let mut store = ExecutionStore::open_in_memory()?;
        store.start_episode(&lineage, &fingerprint)?;
        let durable = DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
        let action = EpisodeLegalAction::new("combat.end-turn-pending", ActionKind::EndTurn)?;
        durable.operation_intent(
            PENDING_OPERATION_ID,
            PENDING_STATE_ID,
            0,
            &action,
            &json!({"kind":"end_turn"}),
            &json!({"state_id":PENDING_STATE_ID,"generation":0,"legal_actions":[
                {"action_id":action.action_id(),"action":{"kind":"end_turn"}}
            ]}),
        )?;
        let digest = durable.operation_payload_digest(PENDING_OPERATION_ID)?;
        durable.operation_dispatched(PENDING_OPERATION_ID, &digest)?;
        let operation = durable
            .pending_operations()?
            .pop()
            .ok_or("missing pending operation")?;
        let encoded = encode_base64(
            operation
                .intent
                .action_payload
                .as_deref()
                .ok_or("missing action bytes")?,
        );
        assert!(
            encoded.ends_with('='),
            "fixture must exercise unpadded gateway bytes"
        );
        let (lookup, reconcile) = settled_frames(
            PENDING_OPERATION_ID,
            PENDING_STATE_ID,
            0,
            &digest,
            operation
                .intent
                .catalog_digest
                .as_deref()
                .ok_or("missing catalog")?,
            encoded.trim_end_matches('='),
        );
        Ok(Self {
            fixture,
            durable,
            lookup,
            reconcile,
        })
    }

    fn run(&self) -> Result<(), String> {
        let mut runtime_config = config("127.0.0.1:15525".into());
        runtime_config.recovery_environment = RecoveryEnvironment::new().0;
        runtime_config.mcp_binary = response_script(&self.fixture, &self.lookup, &self.reconcile)
            .map_err(|error| error.to_string())?;
        let mut port = RuntimeV3Port::new_with_store(
            runtime_config,
            TelemetryHandle::disabled(),
            self.durable.clone(),
        )?;
        port.recovery_authority = Some(recovery_authority());
        port.reconcile_pending_operations()
    }

    fn assert_requests(&self, reconciled: bool) -> Result<(), Box<dyn std::error::Error>> {
        let requests = std::fs::read_to_string(self.fixture.0.join("requests"))?;
        let calls: Vec<Value> = requests
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        let names: Vec<&str> = calls
            .iter()
            .filter_map(|value| value["params"]["name"].as_str())
            .collect();
        let expected = if reconciled {
            vec!["watchdog.operation_lookup", "watchdog.operation_reconcile"]
        } else {
            vec!["watchdog.operation_lookup"]
        };
        assert_eq!(
            names, expected,
            "recovery must never poll or dispatch ordinary gameplay"
        );
        let mut original = None;
        for call in calls.iter().filter(|value| value["method"] == "tools/call") {
            let reference = &call["params"]["arguments"]["payload"]["operation"];
            assert_eq!(reference["operation_id"], PENDING_OPERATION_ID);
            assert_eq!(
                reference["payload_digest"],
                self.durable
                    .operation_payload_digest(PENDING_OPERATION_ID)?
            );
            if let Some(first) = original {
                assert_eq!(reference, first, "reconcile must keep the lookup identity");
            } else {
                original = Some(reference);
            }
        }
        Ok(())
    }
}

fn set_state(frame: &mut Value, state: &str) {
    frame["payload"]["result"]["status"] = json!(state);
    frame["payload"]["operation"]["state"] = json!(state);
}

#[test]
fn subprocess_recovery_resolves_unknown_and_accepts_retained_terminal_states()
-> Result<(), Box<dyn std::error::Error>> {
    for (lookup, reconciled) in [
        ("UNKNOWN", "RECONCILED"),
        ("ACCEPTED", "RECONCILED"),
        ("MAY_HAVE_BEEN_DISPATCHED", "RECONCILED"),
        ("INTENT_RECORDED", "RECONCILED"),
        ("SETTLED", "SETTLED"),
        ("RECONCILED", "RECONCILED"),
        ("REJECTED", "REJECTED"),
    ] {
        let mut case = RecoveryCase::new()?;
        set_state(&mut case.lookup, lookup);
        set_state(&mut case.reconcile, reconciled);
        if reconciled == "REJECTED" {
            for frame in [&mut case.lookup, &mut case.reconcile] {
                frame["payload"]["operation"]["ticket"]["state"] = json!("REJECTED");
                frame["payload"]["operation"]["witness"] = Value::Null;
            }
            case.reconcile["payload"]["witness"] = Value::Null;
        }
        if let Err(error) = case.run() {
            diagnostics::emit_failure(
                &case.fixture.0.join("requests"),
                &case.fixture.0.join("child-status"),
                diagnostics::RecoveryCaseTag::retained_terminal(lookup, reconciled),
                case.durable.operation_state(PENDING_OPERATION_ID),
            );
            return Err(error.into());
        }
        assert_eq!(
            case.durable.operation_state(PENDING_OPERATION_ID)?,
            OperationState::Reconciled
        );
        case.assert_requests(true)?;
    }
    Ok(())
}

#[test]
fn subprocess_missing_or_unresolved_evidence_does_not_poll_gameplay()
-> Result<(), Box<dyn std::error::Error>> {
    for (lookup, reconcile) in [
        ("NOT_FOUND", "RECONCILED"),
        ("UNKNOWN", "UNKNOWN"),
        ("UNKNOWN", "NOT_FOUND"),
    ] {
        let mut case = RecoveryCase::new()?;
        set_state(&mut case.lookup, lookup);
        set_state(&mut case.reconcile, reconcile);
        if lookup == "NOT_FOUND" {
            case.lookup["payload"]["operation"] = Value::Null;
        }
        if reconcile == "NOT_FOUND" {
            case.reconcile["payload"]["operation"] = Value::Null;
            case.reconcile["payload"]["witness"] = Value::Null;
        }
        let result = case.run();
        if let Err(error) = &result {
            diagnostics::emit_failure(
                &case.fixture.0.join("requests"),
                &case.fixture.0.join("child-status"),
                diagnostics::RecoveryCaseTag::unresolved(lookup, reconcile),
                case.durable.operation_state(PENDING_OPERATION_ID),
            );
            assert!(!error.is_empty(), "recovery failure must remain observable");
        }
        assert!(result.is_err());
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(lookup != "NOT_FOUND")?;
    }
    Ok(())
}

#[test]
fn subprocess_missing_or_cross_operation_witness_never_closes_durable_uncertainty()
-> Result<(), Box<dyn std::error::Error>> {
    for defect in [
        "missing",
        "other_operation",
        "other_context",
        "other_fence",
        "generation",
        "duplicate_witness",
    ] {
        let mut case = RecoveryCase::new()?;
        set_state(&mut case.lookup, "UNKNOWN");
        let witness = &mut case.reconcile["payload"]["operation"]["witness"];
        match defect {
            "missing" => *witness = Value::Null,
            "other_operation" => witness["operation_id"] = json!(PENDING_STATE_ID),
            "other_context" => witness["boot_id"] = json!(PENDING_STATE_ID),
            "other_fence" => witness["host_fence_id"] = json!(PENDING_STATE_ID),
            "generation" => witness["generation"] = json!(0),
            _ => {}
        }
        if defect != "duplicate_witness" {
            case.reconcile["payload"]["witness"] = witness.clone();
        } else {
            case.reconcile["payload"]["witness"]["witness_id"] = json!(PENDING_STATE_ID);
        }
        let result = case.run();
        if let Err(error) = &result {
            diagnostics::emit_failure(
                &case.fixture.0.join("requests"),
                &case.fixture.0.join("child-status"),
                diagnostics::RecoveryCaseTag::witness_defect(defect),
                case.durable.operation_state(PENDING_OPERATION_ID),
            );
            assert!(!error.is_empty(), "recovery failure must remain observable");
        }
        assert!(result.is_err(), "accepted {defect}");
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(true)?;
    }
    Ok(())
}
