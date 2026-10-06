// SPDX-License-Identifier: MIT

// Durable draft-state CAS and mutation-receipt replay helpers.

impl Owner {
    fn load_draft_state(
        &self,
        entry: &Current,
        workflow_run_id: &str,
    ) -> Result<(OwnerDraftState, u64), ManagementError> {
        let Some(DurableOwnerContextState {
            record_version,
            bytes,
        }) = entry
            .store
            .load_owner_context_state(&self.configuration.owner_id)
            .map_err(store_error)?
        else {
            return Ok((
                OwnerDraftState::empty(&self.configuration.owner_id, workflow_run_id),
                0,
            ));
        };
        let state: OwnerDraftState = serde_json::from_slice(&bytes).map_err(|_| state_corrupt())?;
        state.validate(&self.configuration.owner_id, workflow_run_id)?;
        Ok((state, record_version))
    }

    fn persist_draft_state(
        &self,
        entry: &mut Current,
        state: &OwnerDraftState,
        expected_version: u64,
    ) -> Result<(), ManagementError> {
        let bytes = state.encode()?;
        entry
            .store
            .compare_exchange_owner_context_state(
                &self.configuration.owner_id,
                expected_version,
                &bytes,
            )
            .map_err(store_error)?;
        Ok(())
    }

    fn existing_receipt(
        &self,
        state: &OwnerDraftState,
        actor: &AuthContext,
        request_id: &str,
        payload_digest: &str,
    ) -> Result<Option<ContextOwnerMutationReceipt>, ManagementError> {
        let Some(receipts) = state.receipts.get(&actor.subject) else {
            return Ok(None);
        };
        let Some(stored) = receipts.get(request_id) else {
            return Ok(None);
        };
        if stored.payload_digest != payload_digest {
            return Err(ManagementError::conflict(
                "context_owner_request_id_reused",
                "request identity was already used with a different canonical payload",
            ));
        }
        Ok(Some(stored.receipt.clone()))
    }

    fn record_receipt(
        &self,
        state: &mut OwnerDraftState,
        receipt: ContextOwnerMutationReceipt,
    ) -> Result<(), ManagementError> {
        let actor_receipts = state
            .receipts
            .entry(receipt.actor_subject.clone())
            .or_default();
        if actor_receipts.len() >= MAX_RECEIPTS_PER_ACTOR
            && !actor_receipts.contains_key(&receipt.request_id)
        {
            return Err(ManagementError::unavailable(
                "context_owner_receipt_capacity",
                "owner mutation receipt history is at its bounded capacity",
            ));
        }
        actor_receipts.insert(
            receipt.request_id.clone(),
            StoredReceipt {
                payload_digest: receipt.payload_digest.clone(),
                receipt,
            },
        );
        Ok(())
    }
}
