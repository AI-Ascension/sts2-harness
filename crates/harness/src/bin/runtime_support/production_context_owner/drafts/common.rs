// SPDX-License-Identifier: MIT

impl Owner {
    fn load_draft_state(
        &self,
        entry: &Current,
        workflow_run_id: &str,
    ) -> Result<(OwnerDraftState, u64), ManagementError> {
        let Some(DurableOwnerContextState { record_version, bytes }) = entry
            .store
            .load_owner_context_state(&self.configuration.owner_id)
            .map_err(store_error)?
        else {
            return Ok((
                OwnerDraftState::empty(&self.configuration.owner_id, workflow_run_id),
                0,
            ));
        };
        let state: OwnerDraftState = serde_json::from_slice(&bytes)
            .map_err(|_| state_corrupt())?;
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

    fn authorize_current_binding(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        entry: &Current,
        require_metadata_read_scope: bool,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        if (require_metadata_read_scope && !actor.can("workflow:read"))
            || !actor.can_run(&snapshot.workflow_run_id)
        {
            return Err(ManagementError::forbidden(
                "context_owner_draft_forbidden",
                "actor cannot read owner context for this workflow run",
            ));
        }
        self.validate_source_entry(actor, snapshot, entry)?;
        let boundary = entry.authority.state().boundary.clone();
        if entry.catalog_generation != Some(boundary.generation)
            || entry.runtime_instance_id.is_empty()
            || entry.runtime_lease_id.is_empty()
            || entry.runtime_lease_epoch == 0
        {
            return Err(ManagementError::unavailable(
                "context_owner_draft_unavailable",
                "current owner observation, epoch, or runtime lease is unavailable",
            ));
        }
        let catalog = self.catalog(actor)?;
        catalog.validate()?;
        self.validate_control_limits_in_catalog(&catalog, &entry.admitted_control_limits)?;
        let request = entry.binding_request.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_binding_unavailable",
                "current owner binding is unavailable",
            )
        })?;
        if request.workflow_run_id != snapshot.workflow_run_id
            || request.definition_digest != snapshot.definition_digest
            || request.instance_id != entry.runtime_instance_id
            || request.graph_id != snapshot.cursor.graph_id
            || request.node_id != snapshot.cursor.node_id
            || request.node_execution_id != snapshot.cursor.node_execution_id
            || request.context_ref != self.configuration.context_ref
            || request.node_kind != "decide"
        {
            return Err(ManagementError::conflict(
                "context_owner_binding_stale",
                "owner draft request is not attached to the current admitted invocation",
            ));
        }
        let binding = self.binding_for_request(request, entry, &catalog)?;
        binding.validate(Some(snapshot))?;
        if require_metadata_read_scope && !binding.grants.metadata_read {
            return Err(ManagementError::capability(
                "context_owner_metadata_unavailable",
                "current binding does not grant context metadata access",
            ));
        }
        Ok(binding)
    }

    fn verify_edit_grant(
        &self,
        actor: &AuthContext,
        binding: &ContextOwnerBinding,
        needs_content: bool,
    ) -> Result<(), ManagementError> {
        if !actor.can("workflow:context:edit") || !binding.grants.edit {
            return Err(ManagementError::forbidden(
                "context_owner_edit_forbidden",
                "current actor and binding must both grant context editing",
            ));
        }
        if needs_content
            && (!actor.can("workflow:context:content:read") || !binding.grants.content_read)
        {
            return Err(ManagementError::forbidden(
                "context_owner_content_forbidden",
                "current actor and binding must both grant bounded context content access",
            ));
        }
        Ok(())
    }

    fn verify_active_source(
        &self,
        entry: &Current,
        record: &StoredDraft,
    ) -> Result<Option<(SourceIdentity, ContextSourceDocument, u64)>, ManagementError> {
        let active = entry
            .store
            .active_context_source(&record.envelope.draft.base_revision_id)
            .map_err(store_error)?;
        let Some((identity, source)) = active else {
            if record.base_source.is_some() {
                return Err(ManagementError::conflict(
                    "context_draft_source_stale",
                    "draft base source is no longer the active owner source",
                ));
            }
            return Ok(None);
        };
        let advertised = self.advertised_source(&identity.source_id)?;
        if advertised.version != identity.version
            || advertised.digest != identity.digest
            || validate_document(&source.document)? != advertised.digest
            || record.base_source.as_ref().is_some_and(|base| {
                base.source_id != identity.source_id
                    || base.version != identity.version
                    || base.digest != identity.digest
            })
        {
            return Err(ManagementError::conflict(
                "context_draft_source_stale",
                "draft no longer has the exact advertised active source it was created from",
            ));
        }
        let valid_until = source_valid_until(&source.document);
        Ok(Some((
            SourceIdentity {
                source_id: identity.source_id,
                version: identity.version,
                digest: identity.digest,
            },
            source.document,
            valid_until,
        )))
    }

    fn eligible_registry(
        &self,
        entry: &Current,
        state: &OwnerDraftState,
        draft: Option<&StoredDraft>,
    ) -> Result<BTreeMap<String, EligibleItem>, ManagementError> {
        let mut sources = self.configuration.sources.clone();
        sources.sort_by(|left, right| left.source_id.cmp(&right.source_id));
        let mut registry = BTreeMap::new();
        for advertised in sources {
            let Some(source) = entry
                .store
                .load_context_source(&advertised.source_id, advertised.version, &advertised.digest)
                .map_err(store_error)?
            else {
                continue;
            };
            if source.source_id != advertised.source_id
                || source.version != advertised.version
                || source.digest != advertised.digest
                || validate_document(&source.document)? != advertised.digest
            {
                return Err(ManagementError::conflict(
                    "context_owner_source_stale",
                    "stored bytes do not match the exact advertised owner source",
                ));
            }
            let identity = SourceIdentity {
                source_id: source.source_id,
                version: source.version,
                digest: source.digest,
            };
            for item in source.document.items.into_values() {
                insert_eligible(&mut registry, item, identity.clone())?;
            }
        }
        if let Some(draft) = draft {
            let mut referenced_authored = draft
                .envelope
                .draft
                .notes
                .iter()
                .map(|note| item_key(&note.reference))
                .collect::<std::collections::BTreeSet<_>>();
            if let Some(objective) = &draft.envelope.draft.objective {
                referenced_authored.insert(item_key(objective));
            }
            for key in draft
                .authored_item_keys
                .iter()
                .filter(|key| referenced_authored.contains(*key))
            {
                let authored = state.authored_items.get(key).ok_or_else(state_corrupt)?;
                if authored.created_by != draft.envelope.actor_subject {
                    return Err(state_corrupt());
                }
                insert_eligible(
                    &mut registry,
                    authored.item.clone(),
                    authored.source.clone(),
                )?;
            }
        }
        Ok(registry)
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
