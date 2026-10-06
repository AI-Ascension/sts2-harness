// SPDX-License-Identifier: MIT
//! Guarded-v2 lookup lifecycle. The terminal decision is withheld until the executor is quiescent.

use super::{Loaded, read_line};
use serde_json::json;
use sts2_harness::ExoDecisionRequest;
use sts2_harness::exo_lookup_process::ExoLookupProfile;
use sts2_harness::exo_lookup_wire::{
    EXO_LOOKUP_BOOTSTRAP_WIRE, EXO_LOOKUP_FEEDBACK_BYTES, EXO_LOOKUP_HISTORY_WIRE, EXO_LOOKUP_WIRE,
    ExoLookupFrame, ExoLookupPayload,
};
use sts2_harness::exo_private_state::{BridgeChildScope, ExecutorPrivateState, GuardedRun};
use tokio::io::AsyncWriteExt;
use tokio::process::Child;

pub(super) async fn relay(
    loaded: &Loaded,
    guarded: &mut GuardedRun,
    credential: String,
    profile: ExoLookupProfile,
) -> Result<Option<Vec<u8>>, &'static str> {
    let mut host_input = tokio::io::stdin();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read_line(&mut host_input),
    )
    .await
    .map_err(|_| "exo_bridge_lookup_timeout")??;
    let start = ExoLookupFrame::parse(&bytes).map_err(|_| "exo_bridge_lookup_frame")?;
    if start.wire_version != profile.wire() {
        return Err("exo_bridge_lookup_profile");
    }
    let ExoLookupPayload::Start {
        request,
        optional_byte_budget,
    } = &start.payload
    else {
        return Err("exo_bridge_lookup_start");
    };
    let feedback_budget = (*optional_byte_budget).min(EXO_LOOKUP_FEEDBACK_BYTES);
    let request: ExoDecisionRequest =
        serde_json::from_value(request.clone()).map_err(|_| "exo_bridge_lookup_request")?;
    request
        .encode(131_072)
        .map_err(|_| "exo_bridge_lookup_request")?;
    if start.sequence != 0 || super::config::unsupported_profile_axis(&request).is_some() {
        return Err("exo_bridge_lookup_profile");
    }

    guarded.verify_quota()?;
    let mut command = super::super::run::guarded_executor_command(loaded, guarded.paths())?;
    command.env(
        "STS2_EXO_LOOKUP_FEEDBACK_BYTES",
        feedback_budget.to_string(),
    );
    if profile != ExoLookupProfile::Terminal {
        command.env("STS2_EXO_LOOKUP_BOOTSTRAP", "1");
    }
    if profile == ExoLookupProfile::History {
        command.env("STS2_EXO_LOOKUP_HISTORY", "1");
    }
    let child_scope = BridgeChildScope::enable()?;
    guarded.begin_spawn()?;
    command.arg(match profile {
        ExoLookupProfile::Terminal => "--lookup",
        ExoLookupProfile::Bootstrap => "--lookup-bootstrap",
        ExoLookupProfile::History => "--lookup-history",
    });
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            guarded.spawn_failed(&child_scope)?;
            return Err("exo_bridge_executor_unavailable");
        }
    };

    let operation = match child.id() {
        Some(pid) => match guarded.record_child(pid) {
            Ok(private) => {
                match guarded.verify_quota().and_then(|_| {
                    guarded_invocation(loaded, &start, &request, &private, credential)
                }) {
                    Ok(invocation) => {
                        drive_child(
                            &mut child,
                            guarded,
                            invocation,
                            start,
                            request,
                            feedback_budget,
                            profile,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        },
        None => Err("exo_bridge_executor_unavailable"),
    };

    if !matches!(&operation, Ok(Some(_))) {
        let _ = child.start_kill();
    }
    let status = match tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await {
        Ok(Ok(status)) => status,
        _ => {
            let _ = child.start_kill();
            tokio::time::timeout(std::time::Duration::from_secs(1), child.wait())
                .await
                .map_err(|_| "exo_bridge_executor_timeout")?
                .map_err(|_| "exo_bridge_executor_failed")?
        }
    };
    let no_descendants = guarded.finish_child(&child_scope)?;
    if !no_descendants {
        return Err("exo_bridge_executor_descendant");
    }
    let terminal = operation?;
    if !status.success() {
        return Err("exo_bridge_executor_failed");
    }
    Ok(terminal)
}

fn guarded_invocation(
    loaded: &Loaded,
    start: &ExoLookupFrame,
    request: &ExoDecisionRequest,
    private: &ExecutorPrivateState,
    credential: String,
) -> Result<Vec<u8>, &'static str> {
    serde_json::to_vec(&json!({
        "version": "sts2.exo-lookup-executor-input-v2",
        "request_id": start.request_id,
        "host_turn_id": start.turn_id,
        "model": loaded.config.model,
        "endpoint": loaded.config.endpoint,
        "module_path": loaded.config.extension,
        "source_root": loaded.config.source_root,
        "state_root": private.paths.state_root,
        "timeout_millis": 115_000,
        "max_output_tokens": 4096,
        "credential": credential,
        "input": {
            "observation": request.observation,
            "legal_action_ids": request.legal_action_ids,
            "objective": request.objective,
            "hard_constraints": request.hard_constraints
        },
        "private_state": private
    }))
    .map_err(|_| "exo_bridge_input")
}

async fn drive_child(
    child: &mut Child,
    guarded: &GuardedRun,
    invocation: Vec<u8>,
    start: ExoLookupFrame,
    request: ExoDecisionRequest,
    feedback_budget: usize,
    profile: ExoLookupProfile,
) -> Result<Option<Vec<u8>>, &'static str> {
    let mut input = child.stdin.take().ok_or("exo_bridge_executor_pipe")?;
    let mut output = child.stdout.take().ok_or("exo_bridge_executor_pipe")?;
    let (incoming, mut feedbacks) = tokio::sync::mpsc::channel(1);
    let reader = tokio::spawn(async move {
        let mut host_input = tokio::io::stdin();
        loop {
            let frame = read_line(&mut host_input).await;
            let failed = frame.is_err();
            if incoming.send(frame).await.is_err() || failed {
                break;
            }
        }
    });
    let mut invocation = invocation;
    invocation.push(b'\n');
    let operation = tokio::time::timeout(std::time::Duration::from_secs(118), async {
        input
            .write_all(&invocation)
            .await
            .map_err(|_| "exo_bridge_executor_pipe")?;
        input
            .flush()
            .await
            .map_err(|_| "exo_bridge_executor_pipe")?;
        let mut quota_poll = tokio::time::interval(std::time::Duration::from_millis(250));
        let mut terminal = None;
        for sequence in 1..=33 {
            let bytes = loop {
                tokio::select! {
                    result = read_line(&mut output) => break result?,
                    _ = feedbacks.recv() => return Err("exo_bridge_lookup_out_of_turn"),
                    _ = quota_poll.tick() => { guarded.verify_quota()?; }
                }
            };
            let frame = ExoLookupFrame::parse(&bytes).map_err(|_| "exo_bridge_lookup_frame")?;
            frame
                .assert_identity(&start.request_id, &start.turn_id, sequence)
                .map_err(|_| "exo_bridge_lookup_identity")?;
            let bootstrap = matches!(frame.payload, ExoLookupPayload::Bootstrap { .. });
            let history = matches!(frame.payload, ExoLookupPayload::History { .. });
            if bootstrap
                && (profile == ExoLookupProfile::Terminal
                    || frame.wire_version != EXO_LOOKUP_BOOTSTRAP_WIRE)
            {
                return Err("exo_bridge_lookup_profile");
            }
            if history
                && (profile != ExoLookupProfile::History
                    || frame.wire_version != EXO_LOOKUP_HISTORY_WIRE)
            {
                return Err("exo_bridge_lookup_profile");
            }
            if !bootstrap && !history && frame.wire_version != EXO_LOOKUP_WIRE {
                return Err("exo_bridge_lookup_profile");
            }
            let is_terminal = match &frame.payload {
                ExoLookupPayload::Decision { action_id }
                    if request.legal_action_ids.contains(action_id) =>
                {
                    true
                }
                ExoLookupPayload::Query { .. }
                | ExoLookupPayload::Bootstrap { .. }
                | ExoLookupPayload::History { .. }
                | ExoLookupPayload::ReadRetained { .. }
                    if sequence <= 32 =>
                {
                    false
                }
                _ => return Err("exo_bridge_lookup_payload"),
            };
            let encoded = frame.encode().map_err(|_| "exo_bridge_lookup_frame")?;
            if is_terminal {
                terminal = Some(encoded);
                break;
            }
            let mut host_output = tokio::io::stdout();
            host_output
                .write_all(&encoded)
                .await
                .map_err(|_| "exo_bridge_output")?;
            host_output.flush().await.map_err(|_| "exo_bridge_output")?;
            let feedback_bytes = loop {
                tokio::select! {
                    feedback = feedbacks.recv() => {
                        break feedback.ok_or("exo_bridge_lookup_input")??;
                    }
                    _ = quota_poll.tick() => { guarded.verify_quota()?; }
                }
            };
            let feedback =
                ExoLookupFrame::parse(&feedback_bytes).map_err(|_| "exo_bridge_lookup_frame")?;
            feedback
                .assert_identity(&start.request_id, &start.turn_id, sequence)
                .map_err(|_| "exo_bridge_lookup_identity")?;
            if bootstrap && feedback.wire_version != EXO_LOOKUP_BOOTSTRAP_WIRE {
                return Err("exo_bridge_lookup_profile");
            }
            if history && feedback.wire_version != EXO_LOOKUP_HISTORY_WIRE {
                return Err("exo_bridge_lookup_profile");
            }
            let ExoLookupPayload::Feedback { value } = &feedback.payload else {
                return Err("exo_bridge_lookup_feedback");
            };
            if serde_json::to_vec(value)
                .map_err(|_| "exo_bridge_lookup_feedback")?
                .len()
                > feedback_budget
            {
                return Err("exo_bridge_lookup_bound");
            }
            input
                .write_all(&feedback.encode().map_err(|_| "exo_bridge_lookup_frame")?)
                .await
                .map_err(|_| "exo_bridge_executor_pipe")?;
            input
                .flush()
                .await
                .map_err(|_| "exo_bridge_executor_pipe")?;
        }
        Ok(terminal)
    })
    .await
    .map_err(|_| "exo_bridge_lookup_timeout")?;
    reader.abort();
    let _ = reader.await;
    drop(input);
    operation
}
