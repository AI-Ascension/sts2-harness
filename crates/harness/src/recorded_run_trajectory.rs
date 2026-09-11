// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::{
    SEED, envelope, identities, observation_summary, privacy_digest, required_str, unknown_evidence,
};

pub(super) fn trajectory_record(row: &Value, ordinal: usize) -> Result<Option<Value>, String> {
    let event = required_str(row, "event")?;
    let source = json!({"stream":"trajectory","record_ordinal":ordinal});
    match event {
        "seeded_run_receipt" => {
            let settled = row
                .pointer("/receipt/settled")
                .ok_or_else(|| String::from("seed receipt omitted settled"))?;
            let identities = identities(
                settled,
                &[
                    "instance_id",
                    "session_id",
                    "lease_id",
                    "operation_id",
                    "correlation_id",
                ],
            )?;
            let requested = settled
                .get("requested_seed")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from("seed receipt omitted requested_seed"))?;
            let canonical_seed = settled
                .get("canonical_seed")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from("seed receipt omitted canonical_seed"))?;
            let generation = settled
                .get("generation")
                .and_then(Value::as_u64)
                .ok_or_else(|| String::from("seed receipt generation invalid"))?;
            let status = required_str(settled, "status")?;
            let run_started = settled
                .pointer("/observation/run_started")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let host_ready = settled
                .pointer("/observation/host_ready")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let witnessed = settled
                .pointer("/effect_witness/kind")
                .and_then(Value::as_str)
                == Some("run_started");
            // `generation` is the request generation.  A settled receipt is valid only when
            // its observation and witness agree on a *later* generation.  This mirrors the
            // runtime-v3 seeded settlement validator; a receipt generation differing from the
            // witness is therefore expected, not an error by itself.
            let observation_generation = settled
                .pointer("/observation/generation")
                .and_then(Value::as_u64);
            let witness_generation = settled
                .pointer("/effect_witness/generation")
                .and_then(Value::as_u64);
            let witness_seed_matches = settled
                .pointer("/effect_witness/canonical_seed")
                .and_then(Value::as_str)
                == Some(canonical_seed);
            let observation_seed_matches = settled
                .pointer("/observation/canonical_seed")
                .and_then(Value::as_str)
                == Some(canonical_seed);
            let settled_ok = super::recorded_run_seed::valid(&row["receipt"])
                && status == "settled"
                && run_started
                && host_ready
                && witnessed
                && observation_generation == witness_generation
                && observation_generation.is_some_and(|value| value > generation)
                && witness_seed_matches
                && observation_seed_matches;
            let admitted_status = if settled_ok { "settled" } else { "unknown" };
            let effect_kind = if settled_ok { "run_started" } else { "unknown" };
            let payload = json!({"profile":SEED,"variant":"seed_start","protocol_version":required_str(settled,"protocol_version")?,"schema_digest":required_str(settled,"schema_digest")?,"context_digest":required_str(settled,"context_digest")?,"generation":generation.to_string(),"run_started":settled_ok,"host_ready":if settled_ok {host_ready} else {false},"effect_kind":effect_kind,"requested_seed_digest":privacy_digest("seed", requested),"canonical_seed_digest":privacy_digest("seed", canonical_seed),"seed_match":requested == canonical_seed,"status":admitted_status});
            let evidence = json!({"process_exit":"unknown","request":if settled_ok {"accepted"} else {"unknown"},"action":if settled_ok {"settled"} else {"unknown"},"outcome":if settled_ok {"observed"} else {"unknown"},"gameplay":"unknown"});
            Ok(Some(envelope(
                "seed_start",
                source,
                identities,
                evidence,
                payload,
            )))
        }
        "model_decision" => {
            let execution = row
                .get("model_execution_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| String::from("model decision id invalid"))?;
            let action = required_str(row, "action_id")?;
            let ids = json!({"model_execution":{"namespace":"seed-readiness.trajectory.model-execution","value":execution.to_string()},"action":{"namespace":"ai-ascension.action.sha256","value":privacy_digest("action",action)}});
            let mut payload = json!({"profile":SEED,"variant":"decision_summary","action_id_digests":[privacy_digest("action",action)],"reused_model_execution":row.get("reused_model_execution").and_then(Value::as_bool).unwrap_or(false)});
            if let Some(observation) = row.get("observation").filter(|v| !v.is_null()) {
                payload["observation"] = observation_summary(observation)?;
            }
            Ok(Some(envelope(
                "decision",
                source,
                ids,
                unknown_evidence(),
                payload,
            )))
        }
        "action_receipt" => {
            if row.get("status").and_then(Value::as_str) != Some("Unknown")
                || row.get("effect") != Some(&Value::Null)
            {
                return Ok(None);
            }
            let mut ids = identities(row, &["operation_id"])?;
            if let Some(action) = row.get("action_id") {
                let action = action
                    .as_str()
                    .ok_or_else(|| String::from("invalid_action_id"))?;
                ids["action"] = json!({"namespace":"ai-ascension.action.sha256","value":privacy_digest("action", action)});
            }
            Ok(Some(envelope(
                "action_outcome",
                source,
                ids,
                unknown_evidence(),
                json!({"profile":SEED,"variant":"action_outcome","status":"unknown"}),
            )))
        }
        "operation_wait_completed" => Ok(Some(envelope(
            "diagnostic",
            source,
            identities(row, &["operation_id"])?,
            unknown_evidence(),
            json!({"profile":SEED,"variant":"diagnostic","code":if event == "action_receipt" {"unadmitted_action_receipt"} else {"operation_wait_completed"}}),
        ))),
        "episode_failed" => Ok(Some(envelope(
            "diagnostic",
            source,
            json!({}),
            json!({"process_exit":"unknown","request":"unknown","action":"unknown","outcome":"unknown","gameplay":"episode_failed"}),
            json!({"profile":SEED,"variant":"diagnostic","code":"episode_failed","value_digest":privacy_digest("unsupported-value",required_str(row,"error_code")?)}),
        ))),
        _ => Ok(Some(envelope(
            "diagnostic",
            source,
            json!({}),
            unknown_evidence(),
            json!({"profile":SEED,"variant":"diagnostic","code":"unsupported_source_event",
                "value_digest":privacy_digest("unsupported-value",event)}),
        ))),
    }
}
