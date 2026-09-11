// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{
    ReceiptQueryIdentity, ReceiptQueryResult, RecoveryError, verify_coop_receipt_query_artifact,
};

use super::super::mcp::McpProcess;
use super::super::runtime_v3_wire as wire;
use super::RuntimeV3Port;

const PROFILE: &str = "coop-receipt-query-v1";
const TOOL: &str = "sts2.coop_receipt_query";

/// Executes one retained-receipt lookup through a freshly initialized neutral MCP profile.
///
/// The operation identity is supplied by the caller and is never reconstructed from the
/// mutable current catalog. This path performs no observation, reconciliation, queueing, or
/// mutation.
pub(super) fn query(
    port: &mut RuntimeV3Port,
    identity: &ReceiptQueryIdentity,
) -> Result<ReceiptQueryResult, RecoveryError> {
    verify_coop_receipt_query_artifact().map_err(|_| RecoveryError::Terminal)?;
    if identity.session_id() != port.config.session_id {
        return Err(RecoveryError::InvalidOperation);
    }
    port.reconnect_for_recovery()?;
    let mut mcp =
        McpProcess::spawn_profile(&port.config, PROFILE).map_err(|_| RecoveryError::PortFailure)?;
    if let Err(error) = wire::initialize_mcp_profile_classified(&mut mcp, PROFILE) {
        let _ = mcp.close();
        return Err(if error.is_transient() {
            RecoveryError::PortFailure
        } else {
            RecoveryError::Terminal
        });
    }
    let call = wire::rpc_call_recovery_read(
        &mut mcp,
        3,
        "tools/call",
        json!({"name": TOOL, "arguments": request_arguments(port, identity)}),
    );
    let result = match call {
        Ok(response) => parse_response(response, port, identity),
        Err(error) if error.is_transient() => Err(RecoveryError::PortFailure),
        Err(_) => Err(RecoveryError::Terminal),
    };
    let close = mcp.close();
    match (result, close) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), _) => Err(error),
        (Ok(_), Err(_)) => Err(RecoveryError::PortFailure),
    }
}

fn request_arguments(port: &RuntimeV3Port, identity: &ReceiptQueryIdentity) -> Value {
    let location = identity.location();
    let coordinate = location
        .coordinate()
        .map(|coordinate| json!({"col": coordinate.col(), "row": coordinate.row()}));
    json!({
        "instance_id": port.config.instance_id,
        "mcp_session_id": port.config.mcp_session_id,
        "lease_id": port.config.lease_id,
        "lease_epoch": port.config.lease_epoch,
        "operation_id": identity.operation_id(),
        "action_kind": identity.action_kind().as_str(),
        "action_fingerprint": identity.action_fingerprint(),
        "run_id": identity.run_id(),
        "location": {
            "act_index": location.act_index(),
            "room_id": location.room_id(),
            "coord": coordinate,
        },
        "actor_id": identity.actor_id(),
        "authority_id": identity.authority_id(),
        "authority_epoch": identity.authority_epoch(),
        "expected_host_generation": identity.expected_host_generation(),
        "before_host_generation": identity.before_host_generation(),
        "participant_ids": identity.participant_ids(),
    })
}

fn parse_response(
    response: Value,
    port: &RuntimeV3Port,
    identity: &ReceiptQueryIdentity,
) -> Result<sts2_harness::ReceiptQueryResult, RecoveryError> {
    let text = response
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
        .ok_or(RecoveryError::Terminal)?;
    ReceiptQueryResult::from_json(
        text,
        identity,
        "3",
        &port.config.instance_id,
        &port.config.session_id,
        &port.config.lease_id,
        port.config.lease_epoch,
    )
    .map_err(|_| RecoveryError::Terminal)
}

#[cfg(test)]
mod tests {
    use super::{PROFILE, TOOL};

    #[test]
    fn profile_is_read_only_and_dedicated() {
        assert_eq!(PROFILE, "coop-receipt-query-v1");
        assert_eq!(TOOL, "sts2.coop_receipt_query");
    }
}
