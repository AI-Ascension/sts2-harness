// SPDX-License-Identifier: MIT

impl FakeRuntimeV2 {
    fn new(context: RuntimeV2Context, observation: RuntimeV2Observation) -> Self {
        Self {
            context,
            observation,
            mutation_count: 0,
            operations: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    fn intervene_with_observation(&mut self, observation: RuntimeV2Observation) {
        self.observation = observation;
    }

    fn state_response(
        &self,
        request: &RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2Error> {
        request.validate()?;
        if request.kind() != RuntimeV2Kind::StateRequest {
            return Err(RuntimeV2Error::Invalid(
                "fake state path received a non-state request",
            ));
        }
        self.require_current_context(request)?;
        RuntimeV2Message::state_response(&self.context, request.correlation_id(), self.observation)
    }

    fn accept_action(
        &mut self,
        request: &RuntimeV2Message,
    ) -> Result<EngineResult, RuntimeV2Error> {
        request.validate()?;
        if request.kind() != RuntimeV2Kind::ActionRequest {
            return Err(RuntimeV2Error::Invalid(
                "fake action path received a non-action request",
            ));
        }
        let operation_id = request
            .operation_id()
            .cloned()
            .ok_or(RuntimeV2Error::Invalid(
                "action request has no operation_id",
            ))?;
        let binding = OperationBinding::from_message(request)?;
        if let Some(stored) = self.operations.get(&operation_id) {
            if stored_binding(stored) != &binding {
                return self.conflict_result(request, operation_id, binding);
            }
            return replay_stored(stored, request);
        }

        if let Some(error_code) = self.fence_error(request) {
            let result = self.rejected_result(
                request,
                operation_id.clone(),
                binding.action.clone(),
                error_code,
            )?;
            self.operations.insert(
                operation_id.clone(),
                StoredOperation::Terminal {
                    binding,
                    result: result.clone(),
                },
            );
            return Ok(EngineResult {
                message: result,
                replayed: false,
            });
        }

        let action = binding.action.clone();
        let accepted = RuntimeV2Message::action_response(
            &self.context,
            request.correlation_id(),
            self.observation.generation,
            operation_id.clone(),
            action,
            Some(self.observation),
            RuntimeV2Status::Accepted,
            None,
            None,
        )?;
        self.operations.insert(
            operation_id.clone(),
            StoredOperation::Admission {
                binding,
                operation_id,
                accepted: accepted.clone(),
                settled: None,
                applied: false,
            },
        );
        Ok(EngineResult {
            message: accepted,
            replayed: false,
        })
    }

    fn disconnect_after_write(
        &mut self,
        operation_id: &RuntimeV2OperationId,
        request: &RuntimeV2Message,
    ) -> Result<DispatchOutcome, RuntimeV2Error> {
        request.validate()?;
        if request.kind() != RuntimeV2Kind::ActionRequest {
            return Err(RuntimeV2Error::Invalid(
                "post-write disconnect received a non-action request",
            ));
        }
        let binding = OperationBinding::from_message(request)?;
        let Some(stored) = self.operations.get(operation_id).cloned() else {
            return Err(RuntimeV2Error::Invalid(
                "post-write disconnect referenced an unknown operation",
            ));
        };
        match stored {
            StoredOperation::Admission {
                binding: stored_binding,
                operation_id: stored_operation_id,
                accepted,
                settled,
                applied,
            } => {
                if stored_operation_id.as_str() != operation_id.as_str()
                    || stored_binding != binding
                {
                    return Err(RuntimeV2Error::Invalid(
                        "post-write disconnect operation context conflicts",
                    ));
                }
                if applied || settled.is_some() {
                    return Err(RuntimeV2Error::Invalid(
                        "post-write disconnect attempted a second mutation",
                    ));
                }
                let next_generation = self
                    .observation
                    .generation
                    .checked_add(1)
                    .ok_or(RuntimeV2Error::Invalid("fake generation overflowed"))?;
                let next_turn = self
                    .observation
                    .turn_index
                    .checked_add(1)
                    .ok_or(RuntimeV2Error::Invalid("fake turn_index overflowed"))?;
                let next_observation = RuntimeV2Observation::new(
                    RuntimeV2CombatPhase::PlayerTurn,
                    next_turn,
                    true,
                    next_generation,
                )?;

                // Admission may be followed by an intervening lease/state change. Recheck the
                // complete live fence immediately before the only mutation in this fake.
                if let Some(error_code) = self.fence_error(request) {
                    let result = self.rejected_result(
                        request,
                        operation_id.clone(),
                        binding.action.clone(),
                        error_code,
                    )?;
                    self.operations.insert(
                        operation_id.clone(),
                        StoredOperation::Terminal {
                            binding,
                            result: result.clone(),
                        },
                    );
                    return Ok(DispatchOutcome::Rejected(result));
                }

                let action = stored_binding.action.clone();
                let settled_message = RuntimeV2Message::action_response(
                    &self.context,
                    request.correlation_id(),
                    next_generation,
                    operation_id.clone(),
                    action,
                    Some(next_observation),
                    RuntimeV2Status::Settled,
                    None,
                    Some(RuntimeV2EffectWitness::turn_end_settled(next_generation)?),
                )?;
                self.observation = next_observation;
                self.mutation_count = self
                    .mutation_count
                    .checked_add(1)
                    .ok_or(RuntimeV2Error::Invalid("fake mutation count overflowed"))?;
                self.operations.insert(
                    operation_id.clone(),
                    StoredOperation::Admission {
                        binding: stored_binding,
                        operation_id: stored_operation_id,
                        accepted,
                        settled: Some(settled_message),
                        applied: true,
                    },
                );
                Err(RuntimeV2Error::PostWriteDisconnect)
            }
            StoredOperation::Terminal { .. } => Err(RuntimeV2Error::Invalid(
                "post-write disconnect referenced a terminal operation",
            )),
        }
    }


}
