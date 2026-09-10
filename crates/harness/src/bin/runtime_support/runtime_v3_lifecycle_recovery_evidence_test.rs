// SPDX-License-Identifier: MIT

use sts2_harness::{
    ActionKind, EpisodeLegalAction, ExecutionFingerprint, ExecutionLineage, ExecutionStore,
    OperationState,
};

use super::super::durable::{DurableHandle, OperationCatalogEvidence};
use super::reconnect_support::*;
use super::*;

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
        let recovery_context = recovery_original_context();
        let catalog = json!([{
            "action_id": action.action_id(),
            "action": {"kind":"end_turn"}
        }]);
        let input = json!({
            "state_id": PENDING_STATE_ID,
            "generation": 0,
            "legal_actions": catalog
        });
        let catalog_raw = serde_json::to_vec(
            input
                .get("legal_actions")
                .ok_or("pending catalog omitted from operation input")?,
        )?;
        durable.operation_intent_with_catalog(
            PENDING_OPERATION_ID,
            PENDING_STATE_ID,
            0,
            &action,
            &json!({"kind":"end_turn"}),
            OperationCatalogEvidence {
                input: &input,
                raw: &catalog_raw,
                original_context: Some(&recovery_context),
            },
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
        enable_historical_recovery(&mut port)?;
        port.allocated = true;
        port.reconcile_pending_operations()
    }

    fn assert_requests(
        &self,
        reconciled: bool,
        reobserved: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let requests = std::fs::read_to_string(self.fixture.0.join("requests"))?;
        let calls: Vec<Value> = requests
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        let names: Vec<&str> = calls
            .iter()
            .filter_map(|value| value["params"]["name"].as_str())
            .collect();
        let mut expected = vec!["watchdog.operation_lookup"];
        if reconciled {
            expected.push("watchdog.operation_reconcile");
        }
        if reobserved {
            expected.push("sts2.reobserve");
        }
        assert_eq!(
            names, expected,
            "recovery must never poll or dispatch ordinary gameplay"
        );
        let mut original = None;
        for call in calls.iter().filter(|value| {
            matches!(
                value["params"]["name"].as_str(),
                Some("watchdog.operation_lookup" | "watchdog.operation_reconcile")
            )
        }) {
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
        case.run()?;
        assert_eq!(
            case.durable.operation_state(PENDING_OPERATION_ID)?,
            OperationState::Reconciled
        );
        case.assert_requests(true, true)?;
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
        assert!(case.run().is_err());
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(lookup != "NOT_FOUND", false)?;
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
        assert!(case.run().is_err(), "accepted {defect}");
        assert!(
            case.durable
                .operation_state(PENDING_OPERATION_ID)?
                .is_unresolved()
        );
        case.assert_requests(true, false)?;
    }
    Ok(())
}
