// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{ActionIdentity, DispatchStatus, OperationState, TransitionReceipt};

use super::super::{RuntimeV3Port, wire};

impl RuntimeV3Port {
    pub(super) fn record_unknown(
        &self,
        operation_id: &str,
        payload_digest: &str,
    ) -> Result<(), sts2_harness::PortError> {
        if let Some(durable) = &self.durable {
            durable
                .operation_result(operation_id, payload_digest, OperationState::Unknown, None)
                .map_err(|error| wire::port_error("dispatch_durability_failed", error, false))?;
        }
        Ok(())
    }

    pub(super) fn persist_dispatch_result(
        &self,
        operation_id: &str,
        payload_digest: &str,
        receipt: &TransitionReceipt,
        response: Option<&Value>,
    ) -> Result<(), sts2_harness::PortError> {
        let Some(durable) = &self.durable else {
            return Ok(());
        };
        let state = match receipt.status() {
            DispatchStatus::Accepted => OperationState::Accepted,
            DispatchStatus::Settled => OperationState::Settled,
            DispatchStatus::Rejected | DispatchStatus::Cancelled => OperationState::Rejected,
            DispatchStatus::Unknown => OperationState::Unknown,
        };
        durable
            .operation_result(
                operation_id,
                payload_digest,
                state,
                (state == OperationState::Settled || state == OperationState::Rejected)
                    .then_some(response)
                    .flatten(),
            )
            .map_err(|error| wire::port_error("dispatch_durability_failed", error, false))
    }

    pub(super) fn finish_durable_dispatch(
        &self,
        identity: &ActionIdentity,
        payload_digest: &str,
        result: Result<TransitionReceipt, sts2_harness::PortError>,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
        match result {
            Ok(receipt) => {
                self.persist_dispatch_result(
                    identity.operation_id.as_str(),
                    payload_digest,
                    &receipt,
                    None,
                )?;
                Ok(receipt)
            }
            Err(error) => {
                self.record_unknown(identity.operation_id.as_str(), payload_digest)?;
                Err(error)
            }
        }
    }
}
