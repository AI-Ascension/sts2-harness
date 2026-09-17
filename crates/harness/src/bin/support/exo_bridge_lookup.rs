// SPDX-License-Identifier: MIT
//! Lookup-only relay: model requests never acquire gameplay or owner-binding authority.
use super::{
    config::Loaded,
    run::{PrivateRoot, executor_command},
};
use serde_json::json;
use sts2_harness::exo_lookup_wire::{
    EXO_LOOKUP_FEEDBACK_BYTES, EXO_LOOKUP_FRAME_BYTES, ExoLookupFrame, ExoLookupPayload,
};
use sts2_harness::{EXO_SOURCE_REVISION, ExoDecisionRequest};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

pub fn execute(loaded: &Loaded, synthetic: bool) -> Result<(), &'static str> {
    execute_mode(loaded, synthetic, false)
}

/// Bootstrap-capable lookup profile. Legacy `--lookup` remains terminal/query-only.
pub fn execute_bootstrap(loaded: &Loaded, synthetic: bool) -> Result<(), &'static str> {
    execute_mode(loaded, synthetic, true)
}

fn execute_mode(
    loaded: &Loaded,
    synthetic: bool,
    bootstrap_capable: bool,
) -> Result<(), &'static str> {
    let credential = if synthetic {
        "sts2-synthetic-model-key".into()
    } else {
        std::env::var("STS2_EXO_MODEL_KEY").map_err(|_| "exo_bridge_credentials_unavailable")?
    };
    if credential.is_empty() {
        return Err("exo_bridge_credentials_unavailable");
    }
    let private = PrivateRoot::create()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "exo_bridge_runtime")?;
    let result = runtime.block_on(relay(loaded, &private, credential, bootstrap_capable));
    // Tokio's process-owned stdin worker cannot interrupt an OS read; main exits after this bound.
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    result
}

async fn read_line(reader: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    loop {
        let byte = reader
            .read_u8()
            .await
            .map_err(|_| "exo_bridge_lookup_input")?;
        if byte == b'\n' {
            return Ok(bytes);
        }
        if bytes.len() >= EXO_LOOKUP_FRAME_BYTES {
            return Err("exo_bridge_lookup_bound");
        }
        bytes.push(byte);
    }
}

async fn relay(
    loaded: &Loaded,
    private: &PrivateRoot,
    credential: String,
    bootstrap_capable: bool,
) -> Result<(), &'static str> {
    let mut host_input = tokio::io::stdin();
    let mut host_output = tokio::io::stdout();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read_line(&mut host_input),
    )
    .await
    .map_err(|_| "exo_bridge_lookup_timeout")??;
    let start = ExoLookupFrame::parse(&bytes).map_err(|_| "exo_bridge_lookup_frame")?;
    if bootstrap_capable
        != (start.wire_version == sts2_harness::exo_lookup_wire::EXO_LOOKUP_BOOTSTRAP_WIRE)
    {
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
    if start.sequence != 0
        || request.provider_revision != EXO_SOURCE_REVISION
        || request.map_context.is_some()
        || request.management_profile.is_some()
        || request.observation.get("protocol_version").is_some()
    {
        return Err("exo_bridge_unsupported_profile");
    }
    let invocation = json!({
        "version":"sts2.exo-lookup-executor-input-v1","request_id":start.request_id,
        "host_turn_id":start.turn_id,"model":loaded.config.model,"endpoint":loaded.config.endpoint,
        "module_path":loaded.config.extension,"source_root":loaded.config.source_root,
        "state_root":private.0.join("state"),"timeout_millis":115_000,"max_output_tokens":4096,
        "credential":credential,"input":{"observation":request.observation,
            "legal_action_ids":request.legal_action_ids,"objective":request.objective,
            "hard_constraints":request.hard_constraints}
    });
    let mut command = executor_command(loaded, private)?;
    command.env(
        "STS2_EXO_LOOKUP_FEEDBACK_BYTES",
        feedback_budget.to_string(),
    );
    if bootstrap_capable {
        command.env("STS2_EXO_LOOKUP_BOOTSTRAP", "1");
    }
    let mut child = command
        .arg(if bootstrap_capable {
            "--lookup-bootstrap"
        } else {
            "--lookup"
        })
        .spawn()
        .map_err(|_| "exo_bridge_executor_unavailable")?;
    let pid = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(rustix::process::Pid::from_raw)
        .ok_or("exo_bridge_executor_unavailable")?;
    let mut input = child.stdin.take().ok_or("exo_bridge_executor_pipe")?;
    let mut output = child.stdout.take().ok_or("exo_bridge_executor_pipe")?;
    let (incoming, mut feedbacks) = tokio::sync::mpsc::channel(1);
    let reader = tokio::spawn(async move {
        loop {
            let frame = read_line(&mut host_input).await;
            let failed = frame.is_err();
            if incoming.send(frame).await.is_err() || failed {
                break;
            }
        }
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(118), async {
        let mut bytes = serde_json::to_vec(&invocation).map_err(|_| "exo_bridge_input")?;
        bytes.push(b'\n');
        input
            .write_all(&bytes)
            .await
            .map_err(|_| "exo_bridge_executor_pipe")?;
        input
            .flush()
            .await
            .map_err(|_| "exo_bridge_executor_pipe")?;
        for sequence in 1..=33 {
            let bytes = tokio::select! {
                result = read_line(&mut output) => result?,
                _ = feedbacks.recv() => return Err("exo_bridge_lookup_out_of_turn"),
            };
            let frame = ExoLookupFrame::parse(&bytes).map_err(|_| "exo_bridge_lookup_frame")?;
            frame
                .assert_identity(&start.request_id, &start.turn_id, sequence)
                .map_err(|_| "exo_bridge_lookup_identity")?;
            let bootstrap = matches!(frame.payload, ExoLookupPayload::Bootstrap { .. });
            if bootstrap
                && (!bootstrap_capable
                    || frame.wire_version
                        != sts2_harness::exo_lookup_wire::EXO_LOOKUP_BOOTSTRAP_WIRE)
            {
                return Err("exo_bridge_lookup_profile");
            }
            if !bootstrap && frame.wire_version != sts2_harness::exo_lookup_wire::EXO_LOOKUP_WIRE {
                return Err("exo_bridge_lookup_profile");
            }
            let terminal = match &frame.payload {
                ExoLookupPayload::Decision { action_id }
                    if request.legal_action_ids.contains(action_id) =>
                {
                    true
                }
                ExoLookupPayload::Query { .. }
                | ExoLookupPayload::Bootstrap { .. }
                | ExoLookupPayload::ReadRetained { .. }
                    if sequence <= 32 =>
                {
                    false
                }
                _ => return Err("exo_bridge_lookup_payload"),
            };
            host_output
                .write_all(&frame.encode().map_err(|_| "exo_bridge_lookup_frame")?)
                .await
                .map_err(|_| "exo_bridge_output")?;
            host_output.flush().await.map_err(|_| "exo_bridge_output")?;
            if terminal {
                return Ok(());
            }
            let feedback =
                ExoLookupFrame::parse(&feedbacks.recv().await.ok_or("exo_bridge_lookup_input")??)
                    .map_err(|_| "exo_bridge_lookup_frame")?;
            feedback
                .assert_identity(&start.request_id, &start.turn_id, sequence)
                .map_err(|_| "exo_bridge_lookup_identity")?;
            if bootstrap
                && feedback.wire_version != sts2_harness::exo_lookup_wire::EXO_LOOKUP_BOOTSTRAP_WIRE
            {
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
        Err("exo_bridge_lookup_bound")
    })
    .await
    .map_err(|_| "exo_bridge_lookup_timeout")
    .and_then(|r| r);
    reader.abort();
    let _ = reader.await;
    drop(input);
    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await;
    result
}
