// SPDX-License-Identifier: MIT

fn response(envelope: &ExoBridgeRequestEnvelope, bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let receipt: Receipt =
        serde_json::from_slice(bytes).map_err(|_| "exo_bridge_invalid_receipt")?;
    validate_receipt(envelope, &receipt)?;
    let terminal = receipt.decision.ok_or("exo_bridge_missing_decision")?;
    validate_decision(envelope, &terminal)?;
    let output = encode_bridge_response(
        &envelope.request_id,
        &envelope.turn_id,
        ExoWireOutcome::Decision,
        Some(terminal.as_bytes()),
        None,
    )
    .map_err(|_| "exo_bridge_invalid_decision")?;
    if output.len() > envelope.request.max_response_bytes as usize {
        return Err("exo_bridge_response_bound");
    }
    Ok(output)
}

fn response_v2(envelope: &ExoBridgeRequestEnvelope, bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let receipt: Receipt =
        serde_json::from_slice(bytes).map_err(|_| "exo_bridge_invalid_receipt")?;
    validate_receipt(envelope, &receipt)?;
    let terminal = receipt
        .decision
        .as_deref()
        .ok_or("exo_bridge_missing_decision")?;
    validate_decision(envelope, terminal)?;
    let decision: Value =
        serde_json::from_str(terminal).map_err(|_| "exo_bridge_invalid_decision")?;
    let response = LifecycleResponse {
        wire_version: "sts2.exo-bridge-wire-v2",
        request_id: &envelope.request_id,
        turn_id: &envelope.turn_id,
        outcome: "decision",
        decision,
        error_code: None,
        native: NativeReceipt {
            agent_id: &receipt.exo_agent_id,
            conversation_id: &receipt.exo_conversation_id,
            session_id: &receipt.exo_session_id,
            turn_id: &receipt.exo_turn_id,
            event_cursor: &receipt.event_cursor,
        },
    };
    let output = serde_json::to_vec(&response).map_err(|_| "exo_bridge_invalid_decision")?;
    if output.len() > envelope.request.max_response_bytes as usize {
        return Err("exo_bridge_response_bound");
    }
    Ok(output)
}

fn validate_receipt(
    envelope: &ExoBridgeRequestEnvelope,
    receipt: &Receipt,
) -> Result<(), &'static str> {
    if receipt.version != "sts2.exo-executor-receipt-v2"
        || receipt.request_id != envelope.request_id
        || receipt.host_turn_id != envelope.turn_id
        || !valid_uuid(&receipt.exo_agent_id)
        || !valid_uuid(&receipt.exo_conversation_id)
        || !valid_uuid(&receipt.exo_turn_id)
        || !valid_uuid(&receipt.exo_session_id)
        || !valid_uuid(&receipt.event_cursor)
    {
        return Err("exo_bridge_receipt_identity");
    }
    if receipt.forwarded_requests > 1
        || receipt.fetch_attempts > 16
        || receipt
            .forwarded_requests
            .checked_add(receipt.denied_requests)
            != Some(receipt.fetch_attempts)
    {
        return Err("exo_bridge_invalid_receipt");
    }
    eprintln!(
        "{}",
        json!({
            "schema": "sts2.exo-one-shot-evidence-v1",
            "exo_agent_id": receipt.exo_agent_id,
            "exo_conversation_id": receipt.exo_conversation_id,
            "exo_turn_id": receipt.exo_turn_id,
            "exo_session_id": receipt.exo_session_id,
            "event_cursor": receipt.event_cursor,
            "fetch_attempts": receipt.fetch_attempts,
            "forwarded_requests": receipt.forwarded_requests,
            "denied_requests": receipt.denied_requests
        })
    );
    if receipt.error_code.is_some()
        || receipt.fetch_attempts != 1
        || receipt.forwarded_requests != 1
    {
        return Err("exo_bridge_executor_failed");
    }
    Ok(())
}

fn validate_decision(
    envelope: &ExoBridgeRequestEnvelope,
    terminal: &str,
) -> Result<(), &'static str> {
    let decision =
        parse_bridge_decision(terminal.as_bytes()).map_err(|_| "exo_bridge_invalid_decision")?;
    let legal = &envelope.request.legal_action_ids;
    match decision {
        Decision::Action { action_id, .. } if !legal.contains(&action_id) => {
            return Err("exo_bridge_illegal_action");
        }
        Decision::Plan { action_ids, .. } if action_ids.iter().any(|id| !legal.contains(id)) => {
            return Err("exo_bridge_illegal_action");
        }
        Decision::Recovery { .. } => return Err("exo_bridge_unsupported_recovery"),
        _ => {}
    }
    Ok(())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok_and(|id| !id.is_nil())
}

pub(super) struct PrivateRoot(pub(super) PathBuf);

impl PrivateRoot {
    pub(super) fn create() -> Result<Self, &'static str> {
        let parent = std::env::temp_dir();
        Self::create_under(&parent, uuid::Uuid::new_v4())
    }

    fn create_under(parent: &Path, identity: uuid::Uuid) -> Result<Self, &'static str> {
        if !parent.is_absolute()
            || !std::fs::symlink_metadata(parent)
                .map_err(|_| "exo_bridge_private_root")?
                .file_type()
                .is_dir()
            || parent
                .canonicalize()
                .map_err(|_| "exo_bridge_private_root")?
                != parent
        {
            return Err("exo_bridge_private_root");
        }
        let path = parent.join(format!("sts2-exo-{identity}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "exo_bridge_private_root")?;
        let root = Self(path);
        for name in ["state", "config", "cache", "temp"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(root.0.join(name))
                .map_err(|_| "exo_bridge_private_root")?;
        }
        Ok(root)
    }

    fn remove(&self) -> Result<(), &'static str> {
        std::fs::remove_dir_all(&self.0).map_err(|_| "exo_bridge_private_cleanup")
    }
}

impl Drop for PrivateRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

