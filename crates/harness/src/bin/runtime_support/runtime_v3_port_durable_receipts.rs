// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    pub(super) fn record_durable_receipt(
        &self,
        operation_id: &str,
        payload_digest: &str,
        receipt: &TransitionReceipt,
        response: &Value,
    ) -> Result<(), String> {
        let Some(durable) = &self.durable else {
            return Ok(());
        };
        let state = match receipt.status() {
            sts2_harness::DispatchStatus::Accepted => sts2_harness::OperationState::Accepted,
            sts2_harness::DispatchStatus::Settled => sts2_harness::OperationState::Settled,
            sts2_harness::DispatchStatus::Rejected
            | sts2_harness::DispatchStatus::Cancelled => sts2_harness::OperationState::Rejected,
            sts2_harness::DispatchStatus::Unknown => sts2_harness::OperationState::Unknown,
        };
        durable.operation_result(operation_id, payload_digest, state, Some(response))
    }

    pub(super) fn record_reconciled_durable_receipt(
        &self,
        operation_id: &str,
        payload_digest: &str,
        receipt: &TransitionReceipt,
        response: &Value,
    ) -> Result<(), String> {
        let Some(durable) = &self.durable else {
            return Ok(());
        };
        match receipt.status() {
            sts2_harness::DispatchStatus::Settled => {
                match durable.operation_state(operation_id)? {
                    sts2_harness::OperationState::Settled
                    | sts2_harness::OperationState::Reconciled => Ok(()),
                    sts2_harness::OperationState::Accepted
                    | sts2_harness::OperationState::Unknown
                    | sts2_harness::OperationState::MayHaveBeenDispatched
                    | sts2_harness::OperationState::IntentRecorded => durable
                        .reconcile_response(
                            operation_id,
                            sts2_harness::OperationState::Settled,
                            response,
                        ),
                    sts2_harness::OperationState::Rejected => Err(String::from(
                        "cannot reconcile a settled receipt after durable rejection",
                    )),
                }
            }
            sts2_harness::DispatchStatus::Rejected | sts2_harness::DispatchStatus::Cancelled => {
                durable.reconcile_response(operation_id, sts2_harness::OperationState::Rejected, response)
            }
            sts2_harness::DispatchStatus::Accepted => {
                let state = durable.operation_state(operation_id)?;
                durable.operation_result(
                    operation_id,
                    payload_digest,
                    if state == sts2_harness::OperationState::Unknown {
                        sts2_harness::OperationState::Unknown
                    } else {
                        sts2_harness::OperationState::Accepted
                    },
                    Some(response),
                )
            }
            sts2_harness::DispatchStatus::Unknown => {
                // Unknown is provisional. Keep the first transport evidence if a read-only
                // reconcile reports Unknown again; recovery responses can carry a new
                // correlation/result digest, but cannot create an evidence conflict or promote
                // the operation to a terminal state.
                match durable.operation_state(operation_id)? {
                    sts2_harness::OperationState::Unknown => Ok(()),
                    // An accepted response is also provisional. Retain it while moving the
                    // operation back to the unresolved Unknown state.
                    sts2_harness::OperationState::Accepted => durable.operation_result(
                        operation_id,
                        payload_digest,
                        sts2_harness::OperationState::Unknown,
                        None,
                    ),
                    _ => durable.operation_result(
                        operation_id,
                        payload_digest,
                        sts2_harness::OperationState::Unknown,
                        Some(response),
                    ),
                }
            }
        }
    }
}
