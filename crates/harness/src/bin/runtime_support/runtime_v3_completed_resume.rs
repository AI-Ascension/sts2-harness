// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{CompletionRecord, CompletionStatus};

use super::super::runtime_v3_telemetry::{
    CleanupStatus, GameOutcome, RuntimeV3Telemetry, TelemetryHandle, TelemetryStage,
};
use super::durable::DurableHandle;

pub(super) fn finish(
    durable: DurableHandle,
    completion: CompletionRecord,
    telemetry_handle: TelemetryHandle,
    telemetry: RuntimeV3Telemetry,
) -> Result<(), String> {
    let output = serialize_completion(&completion)?;
    let close = durable.close();
    drop(durable);
    let (outcome, stage) = match completion.status {
        CompletionStatus::Completed => (GameOutcome::Success, TelemetryStage::Unknown),
        CompletionStatus::Failed => (GameOutcome::Failure, TelemetryStage::Unknown),
        CompletionStatus::Quarantined => (GameOutcome::Unavailable, TelemetryStage::Unknown),
    };
    let close_status = if close.is_ok() {
        CleanupStatus::Clean
    } else {
        CleanupStatus::Failed
    };
    let _ = telemetry_handle.run_finished(outcome, stage, close_status);
    super::finish_telemetry(telemetry);
    close?;
    println!("{output}");
    Ok(())
}

/// Serializes the durable completion record without projecting it into the live-run report
/// shape. A resumed terminal episode has no new observation or runner report to contribute.
pub(super) fn serialize_completion(completion: &CompletionRecord) -> Result<String, String> {
    serde_json::to_string(&completion_value(completion))
        .map_err(|error| format!("runtime-v3 completion serialization failed: {error}"))
}

fn completion_value(completion: &CompletionRecord) -> Value {
    json!({
        "lineage": {
            "run_id": completion.lineage.run_id,
            "episode_id": completion.lineage.episode_id,
            "attempt_id": completion.lineage.attempt_id,
            "trajectory_id": completion.lineage.trajectory_id,
        },
        "status": completion_status(completion.status),
        "terminal_ref": completion.terminal_ref,
        "checkpoint_sequence": completion.checkpoint_sequence,
        "result_digest": completion.result_digest,
    })
}

fn completion_status(status: CompletionStatus) -> &'static str {
    match status {
        CompletionStatus::Completed => "completed",
        CompletionStatus::Failed => "failed",
        CompletionStatus::Quarantined => "quarantined",
    }
}

#[cfg(test)]
mod tests {
    use super::serialize_completion;
    use sts2_harness::{CompletionRecord, CompletionStatus, ExecutionLineage};

    #[test]
    fn serialized_resume_completion_contains_only_the_durable_record() -> Result<(), String> {
        let completion = CompletionRecord::new(
            ExecutionLineage::new("run-1", "episode-1", "attempt-1", "trajectory-1")
                .map_err(|error| format!("lineage is invalid: {error}"))?,
            CompletionStatus::Completed,
            "terminal-victory-3",
            7,
            "result-digest",
        )
        .map_err(|error| format!("completion is invalid: {error}"))?;
        let serialized = serialize_completion(&completion)?;
        let value: serde_json::Value = serde_json::from_str(&serialized)
            .map_err(|error| format!("serialized completion is not JSON: {error}"))?;
        assert_eq!(
            value,
            serde_json::json!({
                "lineage": {
                    "run_id": "run-1",
                    "episode_id": "episode-1",
                    "attempt_id": "attempt-1",
                    "trajectory_id": "trajectory-1",
                },
                "status": "completed",
                "terminal_ref": "terminal-victory-3",
                "checkpoint_sequence": 7,
                "result_digest": "result-digest",
            })
        );
        Ok(())
    }
}
