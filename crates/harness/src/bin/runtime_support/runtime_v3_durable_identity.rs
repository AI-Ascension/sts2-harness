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
                    return Err(self.quarantine_failure(
                        format!(
                            "runtime-v3 operation {} has no canonical action payload",
                            operation.intent.operation_id
                        ),
                        "legacy operation has no canonical action payload; recovery is blocked",
                    ));
                }
            };
            let envelope: Value = serde_json::from_slice(bytes).map_err(|_| {
                self.quarantine_failure(
                    format!(
                        "runtime-v3 operation {} has malformed canonical action payload",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload is malformed",
                )
            })?;
            let object = envelope.as_object().ok_or_else(|| {
                self.quarantine_failure(
                    format!(
                        "runtime-v3 operation {} has a non-object canonical action payload",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload is not an object",
                )
            })?;
            if object.len() != 2
                || object.get("action_id").and_then(Value::as_str)
                    != Some(operation.intent.action_id.as_str())
            {
                return Err(self.quarantine_failure(
                    format!(
                        "runtime-v3 operation {} has inconsistent canonical action identity",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action identity is inconsistent",
                ));
            }
            let payload = object.get("action").ok_or_else(|| {
                self.quarantine_failure(
                    format!(
                        "runtime-v3 operation {} has incomplete canonical action payload",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload is incomplete",
                )
            })?;
            let action = super::super::super::runtime_v3_parse::action_from_payload(
                &operation.intent.action_id,
                payload,
            )
            .map_err(|error| {
                self.quarantine_failure(
                    format!(
                        "runtime-v3 operation {} has invalid canonical action payload: {error}",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload failed validation",
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
                    self.quarantine_failure(
                        format!(
                            "runtime-v3 operation {} action digest failed: {error}",
                            operation.intent.operation_id
                        ),
                        "durable operation canonical action digest could not be calculated",
                    )
                })? != operation.intent.payload_digest
            {
                return Err(self.quarantine_failure(
                    format!(
                        "runtime-v3 operation {} has mismatched canonical action identity",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action kind or digest does not match",
                ));
            }
        }
        Ok(())
    }
}
