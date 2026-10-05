// SPDX-License-Identifier: MIT

impl Owner {
    pub(super) fn recover_mutation_receipt_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerMutationRequest,
    ) -> Result<Option<ContextOwnerMutationReceipt>, ManagementError> {
        if !actor.can("workflow:read") || !actor.can_run(&snapshot.workflow_run_id) {
            return Err(ManagementError::forbidden("context_owner_receipt_forbidden", "actor cannot read owner mutation receipts for this run"));
        }
        let (request_id, payload_digest) = validate_mutation_lookup(request)?;
        let store = ContextControlStore::open(
            scoped_store_path(&self.configuration.store_path, &snapshot.workflow_run_id),
            self.key,
            &snapshot.workflow_run_id,
        ).map_err(store_error)?;
        let durable = store.load_owner_context_state(&self.configuration.owner_id).map_err(store_error)?;
        let Some(durable) = durable else { return Ok(None); };
        let state: OwnerDraftState = serde_json::from_slice(&durable.bytes).map_err(|_| state_corrupt())?;
        state.validate(&self.configuration.owner_id, &snapshot.workflow_run_id)?;
        self.existing_receipt(&state, actor, request_id, &payload_digest)
    }
}
