// SPDX-License-Identifier: MIT

impl FakeRuntimeV2 {

    fn reconcile_action(
        &mut self,
        request: &RuntimeV2Message,
    ) -> Result<EngineResult, RuntimeV2Error> {
        request.validate()?;
        if request.kind() != RuntimeV2Kind::ReconcileRequest {
            return Err(RuntimeV2Error::Invalid(
                "fixed reconcile_action received a non-reconcile request",
            ));
        }
        self.require_current_context(request)?;
        let operation_id = request
            .operation_id()
            .cloned()
            .ok_or(RuntimeV2Error::Invalid(
                "reconcile request has no operation_id",
            ))?;
        let Some(stored) = self.operations.get(&operation_id) else {
            return Err(RuntimeV2Error::Invalid(
                "reconcile request referenced an unknown operation",
            ));
        };
        match stored {
            StoredOperation::Admission {
                binding,
                operation_id,
                accepted,
                settled,
                ..
            } => {
                if binding.generation != request.generation() {
                    return Err(RuntimeV2Error::Invalid(
                        "reconcile request generation conflicts with its operation",
                    ));
                }
                let Some(settled) = settled else {
                    let unknown = RuntimeV2Message::reconcile_response(
                        &self.context,
                        request.correlation_id(),
                        request.generation(),
                        operation_id.clone(),
                        accepted
                            .action()
                            .cloned()
                            .ok_or(RuntimeV2Error::Invalid("accepted result has no action"))?,
                        None,
                        RuntimeV2Status::Unknown,
                        Some("sts2.runtime/operation_unsettled".to_owned()),
                        None,
                    )?;
                    return Ok(EngineResult {
                        message: unknown,
                        replayed: false,
                    });
                };
                let settled_observation = settled.observation().ok_or(RuntimeV2Error::Invalid(
                    "stored settled result has no observation",
                ))?;
                let settled_witness = settled.effect_witness().ok_or(RuntimeV2Error::Invalid(
                    "stored settled result has no witness",
                ))?;
                let result = RuntimeV2Message::reconcile_response(
                    &self.context,
                    request.correlation_id(),
                    settled.generation(),
                    operation_id.clone(),
                    settled.action().cloned().ok_or(RuntimeV2Error::Invalid(
                        "stored settled result has no action",
                    ))?,
                    Some(settled_observation),
                    RuntimeV2Status::Settled,
                    None,
                    Some(settled_witness.clone()),
                )?;
                Ok(EngineResult {
                    message: result,
                    replayed: false,
                })
            }
            StoredOperation::Terminal { result, .. } => {
                let action = result
                    .action()
                    .cloned()
                    .ok_or(RuntimeV2Error::Invalid("terminal result has no action"))?;
                let terminal_result = RuntimeV2Message::reconcile_response(
                    &self.context,
                    request.correlation_id(),
                    result.generation(),
                    operation_id,
                    action,
                    result.observation(),
                    result
                        .status()
                        .ok_or(RuntimeV2Error::Invalid("terminal result has no status"))?,
                    result.error_code().map(str::to_owned),
                    result.effect_witness().cloned(),
                )?;
                Ok(EngineResult {
                    message: terminal_result,
                    replayed: true,
                })
            }
        }
    }

    fn conflict_result(
        &self,
        request: &RuntimeV2Message,
        operation_id: RuntimeV2OperationId,
        binding: OperationBinding,
    ) -> Result<EngineResult, RuntimeV2Error> {
        let result = self.rejected_result(
            request,
            operation_id,
            binding.action,
            "idempotency_conflict",
        )?;
        Ok(EngineResult {
            message: result,
            replayed: false,
        })
    }

    fn rejected_result(
        &self,
        request: &RuntimeV2Message,
        operation_id: RuntimeV2OperationId,
        action: RuntimeV2Action,
        error_code: &str,
    ) -> Result<RuntimeV2Message, RuntimeV2Error> {
        let context = request.context();
        RuntimeV2Message::action_response(
            &context,
            request.correlation_id(),
            self.observation.generation,
            operation_id,
            action,
            Some(self.observation),
            RuntimeV2Status::Rejected,
            Some(error_code.to_owned()),
            None,
        )
    }

    fn require_current_context(&self, request: &RuntimeV2Message) -> Result<(), RuntimeV2Error> {
        if request.instance_id() != self.context.instance_id()
            || request.session_id() != self.context.session_id()
            || request.lease_id() != self.context.lease_id()
            || request.lease_epoch() != self.context.lease_epoch()
        {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 request identity is fenced",
            ));
        }
        Ok(())
    }

    fn fence_error(&self, request: &RuntimeV2Message) -> Option<&'static str> {
        if request.instance_id() != self.context.instance_id()
            || request.session_id() != self.context.session_id()
        {
            return Some("sts2.gateway/stale_identity");
        }
        if request.lease_id() != self.context.lease_id() {
            return Some("sts2.gateway/stale_lease");
        }
        if request.lease_epoch() != self.context.lease_epoch() {
            return Some(STALE_EPOCH_ERROR);
        }
        if request.generation() != self.observation.generation {
            return Some("sts2.game-core/stale_generation");
        }
        if self.observation.combat_phase == RuntimeV2CombatPhase::OutsideCombat {
            return Some("sts2.game-core/outside_combat");
        }
        if self.observation.combat_phase == RuntimeV2CombatPhase::EnemyTurn {
            return Some("sts2.game-core/not_player_turn");
        }
        if !self.observation.host_ready {
            return Some("sts2.gateway/host_not_ready");
        }
        None
    }

    fn mutation_count(&self) -> u16 {
        self.mutation_count
    }
}
