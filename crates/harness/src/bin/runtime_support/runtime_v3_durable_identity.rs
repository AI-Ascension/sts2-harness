// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::DurableHandle;

impl DurableHandle {
    /// Validates every retained mutation before a resumed runtime is allowed to allocate a
    /// gateway lease or start an MCP process. Legacy rows have no canonical action and therefore
    /// enter the durable interrupted-unknown state instead of being guessed or retried.
    pub(in super::super) fn validate_pending_action_identity(&self) -> Result<(), String> {
        let pending = self.pending_operations()?;
        for operation in pending {
            let bytes = match operation.intent.action_payload.as_deref() {
                Some(bytes) => bytes,
                None => {
                    self.mark_interrupted_unknown(
                        "legacy operation has no canonical action payload; recovery is blocked",
                    );
                    return Err(format!(
                        "runtime-v3 operation {} has no canonical action payload",
                        operation.intent.operation_id
                    ));
                }
            };
            let envelope: Value = serde_json::from_slice(bytes).map_err(|_| {
                self.mark_interrupted_unknown(
                    "durable operation canonical action payload is malformed",
                );
                format!(
                    "runtime-v3 operation {} has malformed canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let object = envelope.as_object().ok_or_else(|| {
                self.mark_interrupted_unknown(
                    "durable operation canonical action payload is not an object",
                );
                format!(
                    "runtime-v3 operation {} has a non-object canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            if object.len() != 2
                || object.get("action_id").and_then(Value::as_str)
                    != Some(operation.intent.action_id.as_str())
            {
                self.mark_interrupted_unknown(
                    "durable operation canonical action identity is inconsistent",
                );
                return Err(format!(
                    "runtime-v3 operation {} has inconsistent canonical action identity",
                    operation.intent.operation_id
                ));
            }
            let payload = object.get("action").ok_or_else(|| {
                self.mark_interrupted_unknown(
                    "durable operation canonical action payload is incomplete",
                );
                format!(
                    "runtime-v3 operation {} has incomplete canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let action = super::super::super::runtime_v3_parse::action_from_payload(
                &operation.intent.action_id,
                payload,
            )
            .map_err(|error| {
                self.mark_interrupted_unknown(
                    "durable operation canonical action payload failed validation",
                );
                format!(
                    "runtime-v3 operation {} has invalid canonical action payload: {error}",
                    operation.intent.operation_id
                )
            })?;
            if operation.intent.action_kind.as_deref()
                != Some(super::super::super::runtime_v3_wire::action_kind_name(
                    action.kind(),
                ))
                || super::super::super::runtime_v3_wire::canonical_action_digest(
                    action.action_id(),
                    payload,
                )
                .map_err(|error| {
                    self.mark_interrupted_unknown(
                        "durable operation canonical action digest could not be calculated",
                    );
                    format!(
                        "runtime-v3 operation {} action digest failed: {error}",
                        operation.intent.operation_id
                    )
                })? != operation.intent.payload_digest
            {
                self.mark_interrupted_unknown(
                    "durable operation canonical action kind or digest does not match",
                );
                return Err(format!(
                    "runtime-v3 operation {} has mismatched canonical action identity",
                    operation.intent.operation_id
                ));
            }
        }
        Ok(())
    }
}
