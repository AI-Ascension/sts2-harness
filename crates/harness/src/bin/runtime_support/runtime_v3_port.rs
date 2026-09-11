// SPDX-License-Identifier: MIT


include!("runtime_v3_port_helpers.rs");

impl RuntimeV3Port {
    #[cfg(test)]
    fn new_with_telemetry(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
    ) -> Result<Self, String> {
        Self::new(config, telemetry, None)
    }

    fn new_with_store(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: durable::DurableHandle,
    ) -> Result<Self, String> {
        Self::new(config, telemetry, Some(durable))
    }

    fn new(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: Option<durable::DurableHandle>,
    ) -> Result<Self, String> {
        let gateway = GatewayClient::new(&config)?;
        Ok(Self {
            config,
            gateway,
            mcp: None,
            seeded_mcp: None,
            seeded_receipt: None,
            expert_mcp: None,
            allocated: false,
            released: false,
            next_rpc_id: 1,
            expert_next_rpc_id: 1,
            generation: 0,
            current_state: None,
            current_actions: None,
            catalog: None,
            catalog_raw: None,
            payloads: BTreeMap::new(),
            rest_selector_actions: None,
            rest_selector_payloads: BTreeMap::new(),
            rest_selector_value: None,
            operations: BTreeMap::new(),
            reconnect_attempts: 0,
            telemetry,
            durable,
            last_response_text: None,
            recovery_authority: None,
            recovery: None,
            recovery_context: None,
            recovery_rpc_id: 1,
        })
    }

    fn durable_handle(&self) -> Option<durable::DurableHandle> {
        self.durable.clone()
    }

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
                // Unknown is a provisional outcome. Keep the first transport evidence when a
                // read-only reconcile reports Unknown again; recovery responses can carry a new
                // correlation/result digest, but they must not create an evidence conflict or
                // promote the operation to a terminal state.
                match durable.operation_state(operation_id)? {
                    sts2_harness::OperationState::Unknown => Ok(()),
                    // An accepted response is also provisional. If a later read cannot prove
                    // settlement, retain that first response while moving the operation back to
                    // the unresolved Unknown state; replacing it with a fresh recovery response
                    // would make the provisional evidence appear contradictory.
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

include!("runtime_v3_port_transport.rs");
