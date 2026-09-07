// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::Digest;
use std::collections::BTreeSet;
use sts2_harness::{
    DispatchStatus, EpisodeObservation, OperationState, RecoveryError, RecoveryPort,
    TransitionReceipt, WaitOutcome,
};

use super::super::config::RuntimeConfig;
use super::super::mcp::McpProcess;
use super::super::runtime_v3_telemetry::{ObservationSource, RecoveryKind};
use super::{RuntimeV3Port, parse, wire};

#[derive(Clone, Debug)]
pub(super) struct RecoveryContext {
    instance_id: String,
    lease_id: String,
    lease_epoch: u64,
    mcp_session_id: String,
    original_context: Value,
    current_fence: Value,
}

impl RecoveryContext {
    fn from_environment(config: &RuntimeConfig) -> Result<Self, String> {
        let deployment_id = required_recovery_env(config, "STS2_RECOVERY_DEPLOYMENT_ID")?;
        let instance_id = required_recovery_env(config, "STS2_RECOVERY_INSTANCE_ID")?;
        let instance_incarnation = required_recovery_env(config, "STS2_RECOVERY_INSTANCE_INCAR")?;
        let boot_id = required_recovery_env(config, "STS2_RECOVERY_BOOT_ID")?;
        let lease_id = required_recovery_env(config, "STS2_RECOVERY_LEASE_ID")?;
        let authority_generation =
            required_recovery_env(config, "STS2_RECOVERY_AUTHORITY_GENERATION")?
                .parse::<u64>()
                .map_err(|_| String::from("STS2_RECOVERY_AUTHORITY_GENERATION is invalid"))?;
        let lease_epoch = required_recovery_env(config, "STS2_RECOVERY_LEASE_EPOCH")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_RECOVERY_LEASE_EPOCH is invalid"))?;
        if !valid_uuid(&deployment_id)
            || !valid_uuid(&instance_id)
            || !valid_uuid_v4(&instance_incarnation)
            || !valid_uuid_v4(&boot_id)
            || !valid_uuid_v4(&lease_id)
            || authority_generation == 0
            || lease_epoch == 0
        {
            return Err(String::from(
                "recovery original context has an invalid identity or generation",
            ));
        }
        let current_fence: Value = serde_json::from_str(&required_recovery_env(
            config,
            "STS2_RECOVERY_CURRENT_FENCE_JSON",
        )?)
        .map_err(|_| String::from("STS2_RECOVERY_CURRENT_FENCE_JSON is not valid JSON"))?;
        validate_fence(
            &current_fence,
            &deployment_id,
            &instance_id,
            &instance_incarnation,
            &boot_id,
            authority_generation,
        )?;
        if config.mcp_session_id.is_empty() {
            return Err(String::from("recovery MCP session identity is empty"));
        }
        Ok(Self {
            instance_id: instance_id.clone(),
            lease_id: lease_id.clone(),
            lease_epoch,
            mcp_session_id: config.mcp_session_id.clone(),
            original_context: json!({
                "deployment_id": deployment_id,
                "instance_id": instance_id,
                "instance_incarnation": instance_incarnation,
                "boot_id": boot_id,
                "authority_generation": authority_generation,
                "lease_id": lease_id,
                "lease_epoch": lease_epoch,
            }),
            current_fence,
        })
    }

    fn operation_ref(&self, operation: &sts2_harness::StoredOperation) -> Result<Value, String> {
        let catalog_digest = operation.intent.catalog_digest.as_deref().ok_or_else(|| {
            String::from("durable operation has no original legal-action catalog digest")
        })?;
        if !valid_uuid_v4(&operation.intent.operation_id)
            || !valid_digest(&operation.intent.payload_digest)
            || !valid_digest(catalog_digest)
            || !valid_uuid(&operation.intent.state_id)
        {
            return Err(String::from(
                "durable operation boundary is incompatible with the recovery sideband",
            ));
        }
        Ok(json!({
            "operation_id": operation.intent.operation_id,
            "payload_digest": operation.intent.payload_digest,
            "original_context": self.original_context,
        }))
    }

    fn reconcile_payload(
        &self,
        operation: &sts2_harness::StoredOperation,
    ) -> Result<Value, String> {
        Ok(json!({
            "operation": self.operation_ref(operation)?,
            "strategy": "receipt_lookup",
            "current_fence": self.current_fence,
        }))
    }
}

fn required_recovery_env(config: &RuntimeConfig, name: &str) -> Result<String, String> {
    // Values are captured in RuntimeConfig before the process environment is scrubbed for child
    // MCP processes.  Falling back to the parent environment preserves the live CLI path while
    // allowing tests and embedded callers to provide an isolated configuration directly.
    config
        .recovery_value(name)
        .map(str::to_owned)
        .or_else(|| std::env::var(name).ok())
        .ok_or_else(|| format!("{name} is required for the recovery sideband"))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23)
                    && (byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        && matches!(value.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
}

fn valid_uuid_v4(value: &str) -> bool {
    valid_uuid(value)
        && value.as_bytes().get(14) == Some(&b'4')
        && value
            .as_bytes()
            .get(19)
            .is_some_and(|byte| matches!(*byte, b'8' | b'9' | b'a' | b'b'))
}

fn validate_fence(
    fence: &Value,
    deployment_id: &str,
    instance_id: &str,
    instance_incarnation: &str,
    boot_id: &str,
    authority_generation: u64,
) -> Result<(), String> {
    let object = fence
        .as_object()
        .ok_or_else(|| String::from("recovery current fence is not an object"))?;
    let expected: BTreeSet<&str> = [
        "host_fence_id",
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "fence_generation",
        "created_at",
    ]
    .into_iter()
    .collect();
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || object.get("deployment_id").and_then(Value::as_str) != Some(deployment_id)
        || object.get("instance_id").and_then(Value::as_str) != Some(instance_id)
        || object.get("instance_incarnation").and_then(Value::as_str) != Some(instance_incarnation)
        || object.get("boot_id").and_then(Value::as_str) != Some(boot_id)
        || object.get("authority_generation").and_then(Value::as_u64) != Some(authority_generation)
        || !object
            .get("host_fence_id")
            .and_then(Value::as_str)
            .is_some_and(valid_uuid_v4)
        || object.get("fence_generation").and_then(Value::as_u64) == Some(0)
        || !object
            .get("created_at")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && value.len() <= 64)
    {
        return Err(String::from("recovery current fence is invalid"));
    }
    Ok(())
}

impl RuntimeV3Port {
    fn ensure_recovery_sideband(&mut self) -> Result<(), String> {
        if self.recovery.as_ref().is_some_and(|mcp| !mcp.is_closed()) {
            return Ok(());
        }
        if let Some(mut previous) = self.recovery.take() {
            previous.close()?;
        }
        let context = self
            .recovery_context
            .clone()
            .map(Ok)
            .unwrap_or_else(|| RecoveryContext::from_environment(&self.config))?;
        let mut mcp = McpProcess::spawn_recovery(
            &self.config,
            &context.instance_id,
            &context.lease_id,
            context.lease_epoch,
        )?;
        if let Err(error) = wire::initialize_recovery_mcp(&mut mcp) {
            let _ = mcp.close();
            return Err(error);
        }
        self.recovery_context = Some(context);
        self.recovery = Some(mcp);
        Ok(())
    }

    fn recovery_call_tool(
        &mut self,
        name: &str,
        expected_kind: &str,
        payload: Value,
    ) -> Result<Value, String> {
        self.ensure_recovery_sideband()?;
        let id = self.recovery_rpc_id;
        self.recovery_rpc_id = self
            .recovery_rpc_id
            .checked_add(1)
            .ok_or_else(|| String::from("recovery MCP request identity exhausted"))?;
        let session = self
            .recovery_context
            .as_ref()
            .ok_or_else(|| String::from("recovery context is unavailable"))?
            .mcp_session_id
            .clone();
        let mcp = self
            .recovery
            .as_mut()
            .ok_or_else(|| String::from("recovery MCP process is unavailable"))?;
        wire::recovery_call(mcp, id, &session, name, expected_kind, payload)
    }

    fn durable_operation(
        &self,
        operation_id: &str,
    ) -> Result<sts2_harness::StoredOperation, String> {
        let durable = self.durable.as_ref().ok_or_else(|| {
            String::from("historical recovery requires the durable operation ledger")
        })?;
        durable
            .pending_operations()?
            .into_iter()
            .find(|operation| operation.intent.operation_id == operation_id)
            .ok_or_else(|| format!("durable operation {operation_id} is not pending"))
    }

    fn recovery_operation_record(
        payload: &Value,
        operation: &sts2_harness::StoredOperation,
    ) -> Result<String, String> {
        let value = payload
            .get("operation")
            .filter(|value| !value.is_null())
            .ok_or_else(|| String::from("recovery response omitted the operation record"))?;
        let object = value
            .as_object()
            .ok_or_else(|| String::from("recovery operation record is not an object"))?;
        if object.get("operation_id").and_then(Value::as_str)
            != Some(operation.intent.operation_id.as_str())
            || object.get("payload_digest").and_then(Value::as_str)
                != Some(operation.intent.payload_digest.as_str())
        {
            return Err(String::from(
                "recovery operation record does not match the original payload digest",
            ));
        }
        let boundary = object
            .get("expected_boundary")
            .and_then(Value::as_object)
            .ok_or_else(|| String::from("recovery operation record omitted expected boundary"))?;
        if boundary.get("state_id").and_then(Value::as_str)
            != Some(operation.intent.state_id.as_str())
            || boundary.get("generation").and_then(Value::as_u64)
                != Some(operation.intent.generation)
            || boundary.get("catalog_digest").and_then(Value::as_str)
                != operation.intent.catalog_digest.as_deref()
        {
            return Err(String::from(
                "recovery operation record does not match the original boundary",
            ));
        }
        let action = object
            .get("action")
            .and_then(Value::as_object)
            .ok_or_else(|| String::from("recovery operation record omitted action identity"))?;
        if action.get("schema_digest").and_then(Value::as_str)
            != Some(wire::RUNTIME_V3_SCHEMA_DIGEST)
            || action.get("payload_digest").and_then(Value::as_str)
                != Some(operation.intent.payload_digest.as_str())
        {
            return Err(String::from(
                "recovery operation record contains an invalid canonical action identity",
            ));
        }
        let encoded = action
            .get("canonical_json_b64")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                String::from("recovery operation record omitted canonical action bytes")
            })?;
        let decoded = decode_base64(encoded).ok_or_else(|| {
            String::from("recovery operation record contains invalid canonical action bytes")
        })?;
        let retained = operation.intent.action_payload.as_deref().ok_or_else(|| {
            String::from("durable operation has no canonical action bytes for recovery")
        })?;
        if decoded.as_slice() != retained
            || format!("{:x}", sha2::Sha256::digest(&decoded)) != operation.intent.payload_digest
        {
            return Err(String::from(
                "recovery operation record canonical action bytes do not match the original digest",
            ));
        }
        object
            .get("state")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| String::from("recovery operation record omitted state"))
    }

    // Reconnect only for recovery reads, never to repeat a dispatch. The episode ledger and
    // configured lease/session survive replacement of a failed MCP transport.
    fn reconnect_for_recovery(&mut self) -> Result<(), RecoveryError> {
        if !self.allocated || self.released {
            return Err(RecoveryError::PortFailure);
        }
        if self.mcp.as_ref().is_some_and(|mcp| !mcp.is_closed()) {
            return Ok(());
        }
        if self.reconnect_attempts >= 2 {
            return Err(RecoveryError::PortFailure);
        }
        self.reconnect_attempts += 1;
        if let Some(mut previous) = self.mcp.take() {
            previous.close().map_err(|_| RecoveryError::PortFailure)?;
        }
        let mut mcp = McpProcess::spawn(&self.config).map_err(|_| RecoveryError::PortFailure)?;
        wire::initialize_mcp(&mut mcp).map_err(|_| RecoveryError::PortFailure)?;
        self.mcp = Some(mcp);
        let _ = self.telemetry.recovery(
            RecoveryKind::Reconnect,
            None,
            self.reconnect_attempts,
            "success",
            None,
        );
        Ok(())
    }

    /// Rehydrates the durable operation ledger after MCP startup and resolves each retained
    /// mutation before the runner can ask the provider for a new decision.
    pub(super) fn reconcile_pending_operations(&mut self) -> Result<(), String> {
        let Some(durable) = self.durable.clone() else {
            return Ok(());
        };
        let pending = durable.pending_operations()?;
        for operation in pending {
            let action_bytes = match operation.intent.action_payload.as_deref() {
                Some(bytes) => bytes,
                None => {
                    durable.mark_interrupted_unknown(
                        "legacy operation has no canonical action payload; recovery is blocked",
                    );
                    return Err(format!(
                        "cannot resume operation {} without its canonical action payload",
                        operation.intent.operation_id
                    ));
                }
            };
            let payload: Value = serde_json::from_slice(action_bytes).map_err(|_| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is malformed",
                );
                format!(
                    "cannot resume operation {} with malformed canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let envelope = payload.as_object().ok_or_else(|| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is not an object",
                );
                format!(
                    "cannot resume operation {} with a non-object canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            if envelope.len() != 2
                || envelope.get("action_id").and_then(Value::as_str)
                    != Some(operation.intent.action_id.as_str())
            {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action identity is inconsistent",
                );
                return Err(format!(
                    "cannot resume operation {} with inconsistent canonical action identity",
                    operation.intent.operation_id
                ));
            }
            let action_payload = envelope.get("action").ok_or_else(|| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is incomplete",
                );
                format!(
                    "cannot resume operation {} with incomplete canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let action = super::parse::action_from_payload(
                &operation.intent.action_id,
                action_payload,
            )
            .map_err(|error| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload failed validation",
                );
                format!(
                    "cannot resume operation {} with invalid canonical action payload: {error}",
                    operation.intent.operation_id
                )
            })?;
            if operation.intent.action_kind.as_deref()
                != Some(super::wire::action_kind_name(action.kind()))
                || super::wire::canonical_action_digest(action.action_id(), action_payload)
                    .map_err(|error| {
                        durable.mark_interrupted_unknown(
                            "durable operation canonical action digest could not be calculated",
                        );
                        format!(
                            "cannot validate operation {} canonical action: {error}",
                            operation.intent.operation_id
                        )
                    })?
                    != operation.intent.payload_digest
            {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action kind or digest does not match",
                );
                return Err(format!(
                    "cannot resume operation {} with mismatched canonical action identity",
                    operation.intent.operation_id
                ));
            }
            self.operations.insert(
                operation.intent.operation_id.clone(),
                super::OperationRecord {
                    state_id: operation.intent.state_id.clone(),
                    generation: operation.intent.generation,
                    action,
                },
            );
            let operation_id = operation.intent.operation_id.as_str();
            let receipt = self.reconcile(operation_id).map_err(|error| {
                format!("cannot reconcile retained operation {operation_id}: {error}")
            })?;
            match receipt.status() {
                DispatchStatus::Settled | DispatchStatus::Rejected | DispatchStatus::Cancelled => {}
                DispatchStatus::Accepted | DispatchStatus::Unknown => {
                    let sample = self
                        .poll_operation(operation_id, 1_000)
                        .map_err(|_| format!("retained operation {operation_id} did not settle"))?;
                    if !matches!(
                        sample.outcome(),
                        WaitOutcome::Successor | WaitOutcome::SameStateMutation
                    ) {
                        return Err(format!(
                            "retained operation {operation_id} remains unresolved"
                        ));
                    }
                }
            }
            let state = durable.operation_state(operation_id)?;
            if state.is_unresolved() {
                return Err(format!(
                    "retained operation {operation_id} remains in durable state {state:?}"
                ));
            }
        }
        durable.refresh_resume_boundary()?;
        Ok(())
    }
}

impl RecoveryPort for RuntimeV3Port {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        self.reconnect_for_recovery()?;
        let value = self
            .call_tool("sts2.reobserve", self.context(self.generation))
            .map_err(|_| RecoveryError::PortFailure)?;
        let parsed = parse::observation(&value, "reobserve_response", &self.config)
            .map_err(|_| RecoveryError::PortFailure)?;
        let observation = self
            .install(parsed)
            .map_err(|_| RecoveryError::PortFailure)?;
        let _ = self
            .telemetry
            .observation(ObservationSource::Reobserve, &observation);
        let _ = self.telemetry.recovery(
            RecoveryKind::Reobserve,
            None,
            self.reconnect_attempts,
            "success",
            None,
        );
        Ok(observation)
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or(RecoveryError::InvalidOperation)?;
        let operation = self
            .durable_operation(operation_id)
            .map_err(|_| RecoveryError::PortFailure)?;
        let context = self
            .recovery_context
            .clone()
            .or_else(|| RecoveryContext::from_environment(&self.config).ok())
            .ok_or(RecoveryError::PortFailure)?;
        let operation_ref = context
            .operation_ref(&operation)
            .map_err(|_| RecoveryError::PortFailure)?;
        let lookup = self
            .recovery_call_tool(
                "watchdog.operation_lookup",
                "operation_lookup_response",
                json!({"operation": operation_ref, "lookup_scope": "historical_read"}),
            )
            .map_err(|_| RecoveryError::PortFailure)?;
        let lookup_payload = lookup.get("payload").ok_or(RecoveryError::PortFailure)?;
        if lookup_payload.get("mutation_authorized") != Some(&Value::Bool(false)) {
            return Err(RecoveryError::PortFailure);
        }
        let lookup_state = Self::recovery_operation_record(lookup_payload, &operation)
            .map_err(|_| RecoveryError::PortFailure)?;
        if lookup_payload
            .get("result")
            .and_then(|result| result.get("status"))
            .and_then(Value::as_str)
            != Some(lookup_state.as_str())
        {
            return Err(RecoveryError::PortFailure);
        }
        let resolved_state = match lookup_state.as_str() {
            "SETTLED" => (OperationState::Settled, DispatchStatus::Settled),
            "REJECTED" => (OperationState::Rejected, DispatchStatus::Rejected),
            "UNKNOWN" | "MAY_HAVE_BEEN_DISPATCHED" | "ACCEPTED" => {
                return Ok(TransitionReceipt::new(
                    operation_id,
                    record.action,
                    DispatchStatus::Unknown,
                    None,
                    None,
                    Some(String::from("recovery_required")),
                ));
            }
            _ => return Err(RecoveryError::PortFailure),
        };
        let reconcile = self
            .recovery_call_tool(
                "watchdog.operation_reconcile",
                "operation_reconcile_response",
                context
                    .reconcile_payload(&operation)
                    .map_err(|_| RecoveryError::PortFailure)?,
            )
            .map_err(|_| RecoveryError::PortFailure)?;
        let reconcile_payload = reconcile.get("payload").ok_or(RecoveryError::PortFailure)?;
        if reconcile_payload
            .get("result")
            .and_then(|result| result.get("status"))
            .and_then(Value::as_str)
            != Some("RECONCILED")
        {
            return Err(RecoveryError::PortFailure);
        }
        Self::recovery_operation_record(reconcile_payload, &operation)
            .map_err(|_| RecoveryError::PortFailure)?;
        if let Some(durable) = &self.durable {
            durable
                .reconcile_response(operation_id, resolved_state.0, &reconcile)
                .map_err(|_| RecoveryError::PortFailure)?;
        }
        let receipt = TransitionReceipt::new(
            operation_id,
            record.action,
            resolved_state.1,
            None,
            None,
            None,
        );
        super::recording::receipt(&receipt, record.generation, &self.telemetry);
        let _ = self.telemetry.recovery(
            RecoveryKind::Reconcile,
            Some(operation_id),
            self.reconnect_attempts,
            "success",
            None,
        );
        Ok(receipt)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        self.release_lease_inner()
            .map_err(|_| RecoveryError::PortFailure)
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        RecoveryPort::release_lease(self)
    }
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return None;
    }
    let padding = value.bytes().filter(|byte| *byte == b'=').count();
    if padding > 2
        || value
            .bytes()
            .take(value.len().saturating_sub(padding))
            .any(|byte| byte == b'=')
    {
        return None;
    }
    let mut encoded = value.as_bytes().to_vec();
    if padding == 0 {
        match encoded.len() % 4 {
            0 => {}
            2 => encoded.extend_from_slice(b"=="),
            3 => encoded.push(b'='),
            _ => return None,
        }
    } else if !encoded.len().is_multiple_of(4) {
        return None;
    }
    let mut decoded = Vec::with_capacity(encoded.len() / 4 * 3);
    for (index, chunk) in encoded.chunks_exact(4).enumerate() {
        let last = index + 1 == encoded.len() / 4;
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            64
        } else {
            base64_value(chunk[2])?
        };
        let d = if chunk[3] == b'=' {
            64
        } else {
            base64_value(chunk[3])?
        };
        if a >= 64 || b >= 64 || (c == 64 && d != 64) || (!last && (c == 64 || d == 64)) {
            return None;
        }
        decoded.push((a << 2) | (b >> 4));
        if c != 64 {
            decoded.push((b << 4) | (c >> 2));
            if d != 64 {
                decoded.push((c << 6) | d);
            }
        }
        if decoded.len() > sts2_harness::MAX_OPERATION_ACTION_BYTES {
            return None;
        }
    }
    Some(decoded)
}

fn base64_value(byte: u8) -> Option<u8> {
    Some(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    })
}
