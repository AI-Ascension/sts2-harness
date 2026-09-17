// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::super::branch_continuation_runtime::SelectedBranchContinuation;
use super::super::super::config::RuntimeConfig;
use super::super::{MAX_CHUNK_BYTES, VerifiedClosure, branch_payload, encode_base64};
use super::transport::{exchange, request_frame};

#[path = "exact_restore_operation_receipt.rs"]
mod receipt;
use super::{is_error_response, not_started, phase_error, uncertain};
use receipt::{classify_error_response, classify_lookup_error_response, verify_receipt};

pub(super) fn run_operation(
    process: &mut Option<super::super::super::mcp_process::McpProcess>,
    rpc_id: &mut u64,
    config: &RuntimeConfig,
    selected: &SelectedBranchContinuation,
    closure: &VerifiedClosure,
    operation_id: &str,
    expected_owner: &Value,
) -> Result<Value, super::super::ExactRestoreError> {
    let begin_payload = closure
        .begin_payload(selected, operation_id, expected_owner.clone())
        .map_err(not_started)?;
    let begin = request_frame("exact_restore_begin_request", begin_payload).map_err(not_started)?;
    let begin_response = exchange(process, rpc_id, config, &begin, "sts2.exact_restore.begin")
        .map_err(|error| phase_error(error, "begin"))?;
    if is_error_response(&begin_response) {
        return Err(classify_error_response(&begin_response, "begin"));
    }
    let state = begin_response["payload"]["state"]
        .as_str()
        .ok_or_else(|| uncertain("begin response omitted current operation state"))?;
    if state == "RESTORE_VERIFIED" {
        return verify_receipt(
            &begin_response["payload"]["receipt"],
            selected,
            closure,
            expected_owner,
            operation_id,
        )
        .cloned();
    }
    if matches!(state, "COMMIT_INTENT" | "UNKNOWN") {
        return lookup_verified_receipt(
            process,
            rpc_id,
            config,
            selected,
            closure,
            operation_id,
            expected_owner,
        );
    }
    if !matches!(state, "STAGING" | "CLOSURE_VERIFIED")
        || !matches!(
            begin_response["payload"]["result"].as_str(),
            Some("CREATED" | "EXISTING")
        )
    {
        return Err(uncertain(
            "begin response did not report an admissible current operation state",
        ));
    }
    if state == "STAGING" {
        for blob in &closure.transfer_blobs {
            transfer_blob(process, rpc_id, config, operation_id, expected_owner, blob)?;
            finish_blob(process, rpc_id, config, operation_id, expected_owner, blob)?;
        }
    }
    commit(
        process,
        rpc_id,
        config,
        selected,
        closure,
        operation_id,
        expected_owner,
    )
}

fn commit(
    process: &mut Option<super::super::super::mcp_process::McpProcess>,
    rpc_id: &mut u64,
    config: &RuntimeConfig,
    selected: &SelectedBranchContinuation,
    closure: &VerifiedClosure,
    operation_id: &str,
    expected_owner: &Value,
) -> Result<Value, super::super::ExactRestoreError> {
    let frame = request_frame(
        "exact_restore_commit_request",
        json!({
            "operation_id": operation_id,
            "expected_owner": expected_owner,
            "branch": branch_payload(selected),
            "checkpoint_id": closure.checkpoint_id,
            "closure_digest": closure.closure_digest,
            "exact_state_digest": closure.exact_state_digest,
            "manifest_digest": closure.manifest_digest,
        }),
    )
    .map_err(not_started)?;
    let response = match exchange(process, rpc_id, config, &frame, "sts2.exact_restore.commit") {
        Ok(response) => response,
        Err(_) => {
            return lookup_verified_receipt(
                process,
                rpc_id,
                config,
                selected,
                closure,
                operation_id,
                expected_owner,
            );
        }
    };
    if is_error_response(&response) {
        let payload = &response["payload"];
        if payload["outcome"] == "UNAVAILABLE" && payload["host_effect"] == "may_have_started" {
            return lookup_verified_receipt(
                process,
                rpc_id,
                config,
                selected,
                closure,
                operation_id,
                expected_owner,
            );
        }
        return Err(classify_error_response(&response, "commit"));
    }
    match response["payload"]["state"].as_str() {
        Some("RESTORE_VERIFIED") => verify_receipt(
            &response["payload"]["receipt"],
            selected,
            closure,
            expected_owner,
            operation_id,
        )
        .cloned(),
        Some("COMMIT_INTENT" | "UNKNOWN") => lookup_verified_receipt(
            process,
            rpc_id,
            config,
            selected,
            closure,
            operation_id,
            expected_owner,
        ),
        _ => Err(uncertain(
            "commit response did not return independently verified destination evidence",
        )),
    }
}

fn transfer_blob(
    process: &mut Option<super::super::super::mcp_process::McpProcess>,
    rpc_id: &mut u64,
    config: &RuntimeConfig,
    operation_id: &str,
    expected_owner: &Value,
    blob: &super::super::TransferBlob,
) -> Result<(), super::super::ExactRestoreError> {
    let mut offset = 0_usize;
    while offset < blob.bytes.len() {
        let end = offset.saturating_add(MAX_CHUNK_BYTES).min(blob.bytes.len());
        let chunk = &blob.bytes[offset..end];
        if chunk.is_empty() {
            return Err(not_started("exact restore does not send empty chunks"));
        }
        let frame = request_frame(
            "exact_restore_chunk_request",
            json!({
                "operation_id": operation_id,
                "expected_owner": expected_owner,
                "artifact_digest": blob.digest,
                "offset": offset,
                "total_bytes": blob.bytes.len(),
                "chunk_digest": format!("sha256:{}", sts2_harness::sha256_hex(chunk)),
                "data_base64": encode_base64(chunk),
            }),
        )
        .map_err(not_started)?;
        let response = exchange(
            process,
            rpc_id,
            config,
            &frame,
            "sts2.exact_restore.put_chunk",
        )
        .map_err(|error| phase_error(error, "chunk"))?;
        if is_error_response(&response) {
            return Err(classify_error_response(&response, "chunk"));
        }
        if response["payload"]["result"] != "CHUNK_ACCEPTED"
            || response["payload"]["artifact_digest"] != blob.digest
            || response["payload"]["next_offset"].as_u64() != u64::try_from(end).ok()
        {
            return Err(uncertain(
                "chunk acknowledgement does not prove contiguous upload progress",
            ));
        }
        offset = end;
    }
    Ok(())
}

fn finish_blob(
    process: &mut Option<super::super::super::mcp_process::McpProcess>,
    rpc_id: &mut u64,
    config: &RuntimeConfig,
    operation_id: &str,
    expected_owner: &Value,
    blob: &super::super::TransferBlob,
) -> Result<(), super::super::ExactRestoreError> {
    let frame = request_frame(
        "exact_restore_finish_blob_request",
        json!({
            "operation_id": operation_id,
            "expected_owner": expected_owner,
            "artifact_digest": blob.digest,
            "total_bytes": blob.bytes.len(),
        }),
    )
    .map_err(not_started)?;
    let response = exchange(
        process,
        rpc_id,
        config,
        &frame,
        "sts2.exact_restore.finish_blob",
    )
    .map_err(|error| phase_error(error, "finish_blob"))?;
    if is_error_response(&response) {
        return Err(classify_error_response(&response, "finish_blob"));
    }
    if response["payload"]["result"] != "BLOB_VERIFIED"
        || response["payload"]["artifact_digest"] != blob.digest
        || response["payload"]["total_bytes"].as_u64() != u64::try_from(blob.bytes.len()).ok()
    {
        return Err(uncertain(
            "finish acknowledgement does not verify the uploaded blob",
        ));
    }
    Ok(())
}

fn lookup_verified_receipt(
    process: &mut Option<super::super::super::mcp_process::McpProcess>,
    rpc_id: &mut u64,
    config: &RuntimeConfig,
    selected: &SelectedBranchContinuation,
    closure: &VerifiedClosure,
    operation_id: &str,
    expected_owner: &Value,
) -> Result<Value, super::super::ExactRestoreError> {
    let frame = request_frame(
        "exact_restore_lookup_request",
        json!({"operation_id": operation_id, "expected_owner": expected_owner}),
    )
    .map_err(not_started)?;
    let response =
        exchange(process, rpc_id, config, &frame, "sts2.exact_restore.lookup").map_err(|_| {
            uncertain(
                "commit acknowledgement is uncertain and same-operation lookup is unavailable",
            )
        })?;
    if is_error_response(&response) {
        return Err(classify_lookup_error_response(&response));
    }
    match response["payload"]["state"].as_str() {
        Some("RESTORE_VERIFIED") => verify_receipt(
            &response["payload"]["receipt"],
            selected,
            closure,
            expected_owner,
            operation_id,
        )
        .cloned(),
        Some("UNKNOWN" | "COMMIT_INTENT" | "NOT_FOUND") => Err(uncertain(
            "exact-restore operation remains unresolved; commit was not retried",
        )),
        _ => Err(uncertain(
            "same-operation lookup did not return a verified receipt",
        )),
    }
}
