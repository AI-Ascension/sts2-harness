// SPDX-License-Identifier: MIT

impl MemoryCorpus {
    pub fn select(
        &self,
        request: &SelectionRequest,
        now: &str,
    ) -> Result<SelectionManifest, MemoryError> {
        request.policy.validate(self)?;
        if !valid_id(&request.selection_id)
            || !valid_id(&request.branch_id)
            || request.cutoff > 9_007_199_254_740_991
            || request.corpus_generation == 0
            || request.corpus_generation > 9_007_199_254_740_991
            || request.corpus_generation != request.policy.corpus_generation
            || request.optional_sources.len() > request.policy.max_candidates
            || request.pinned_entry_ids.len() > request.policy.max_selected
            || request.pinned_entry_ids.iter().any(|id| !valid_id(id))
            || request.pinned_entry_ids.iter().collect::<BTreeSet<_>>().len()
                != request.pinned_entry_ids.len()
            || !valid_digest(&request.mandatory_manifest_sha256)
            || !valid_digest(&request.phase2_prepared_manifest_sha256)
            || request.mandatory_bytes.len() > MAX_JOB_INPUT_BYTES
            || sha256_hex(&request.mandatory_bytes) != request.mandatory_manifest_sha256
            || !valid_timestamp(&request.expires_at)
            || !valid_timestamp(now)
            || request.expires_at.as_str() <= now
        {
            return Err(MemoryError::InvalidQuery);
        }
        let mut selected = Vec::new();
        let mut exclusions = Vec::new();
        let mut seen_sources = BTreeSet::new();
        for reference in &request.optional_sources {
            if !seen_sources.insert(reference.clone()) {
                exclusions.push(ExclusionReason {
                    entry_id: reference.entry_id.clone(),
                    reason: "duplicate_source".to_owned(),
                });
                continue;
            }
            let Some(entry) = self.entries.get(reference) else {
                exclusions.push(ExclusionReason {
                    entry_id: reference.entry_id.clone(),
                    reason: "missing_source".to_owned(),
                });
                continue;
            };
            if request.policy.mode == PolicyMode::BoundedPerDecision
                && !request.policy.approved_summary_catalog.contains(reference)
            {
                exclusions.push(ExclusionReason {
                    entry_id: reference.entry_id.clone(),
                    reason: "catalog_not_approved".to_owned(),
                });
                continue;
            }
            if self
                .eligible_entry(
                    entry,
                    &request.branch_id,
                    request.cutoff,
                    request.corpus_generation,
                    now,
                    false,
                )
                .is_err()
            {
                exclusions.push(ExclusionReason {
                    entry_id: reference.entry_id.clone(),
                    reason: "ineligible_source".to_owned(),
                });
                continue;
            }
            selected.push(reference.clone());
        }
        selected.sort();
        let pinned = request
            .pinned_entry_ids
            .iter()
            .collect::<BTreeSet<_>>();
        for pin in &request.pinned_entry_ids {
            if !selected.iter().any(|reference| &reference.entry_id == pin) {
                return Err(MemoryError::PinSelection);
            }
        }
        if pinned.len() > request.policy.max_selected {
            return Err(MemoryError::BudgetExceeded);
        }
        if selected.len() > request.policy.max_selected {
            selected.sort_by_key(|reference| {
                (
                    !pinned.contains(&reference.entry_id),
                    reference.clone(),
                )
            });
            for reference in selected.iter().skip(request.policy.max_selected) {
                exclusions.push(ExclusionReason {
                    entry_id: reference.entry_id.clone(),
                    reason: "max_selected".to_owned(),
                });
            }
            selected.truncate(request.policy.max_selected);
        }
        let mut optional_bytes = Vec::new();
        let mut kept = Vec::new();
        let available_whole_budget = MAX_JOB_INPUT_BYTES.saturating_sub(request.mandatory_bytes.len());
        let optional_limit = request

            .policy
            .optional_byte_budget
            .min(available_whole_budget);
        for reference in selected {
            let entry = self
                .entries
                .get(&reference)
                .ok_or(MemoryError::MissingParent)?;
            if optional_bytes.len().saturating_add(entry.content.len()) > optional_limit {
                if request
                    .pinned_entry_ids
                    .iter()
                    .any(|id| id == &reference.entry_id)
                {
                    return Err(MemoryError::BudgetExceeded);
                }
                exclusions.push(ExclusionReason {
                    entry_id: reference.entry_id.clone(),
                    reason: if optional_limit < request.policy.optional_byte_budget {
                        "whole_input_budget".to_owned()
                    } else {
                        "optional_byte_budget".to_owned()
                    },
                });
                continue;
            }
            optional_bytes.extend_from_slice(&entry.content);
            kept.push(reference);
        }
        let mut rendered = Vec::new();
        rendered.extend_from_slice(b"phase2-memory-v1\n");
        rendered.extend_from_slice(&request.mandatory_bytes);
        rendered.extend_from_slice(b"\n--historical--\n");
        rendered.extend_from_slice(&optional_bytes);
        if rendered.len() > MAX_JOB_INPUT_BYTES {
            return Err(MemoryError::MandatoryOverflow);
        }
        let prepared_digest = sha256_hex(&rendered);
        Ok(SelectionManifest {
            schema: MEMORY_SELECTION_SCHEMA.to_owned(),
            selection_id: request.selection_id.clone(),
            scope: self.scope.clone(),
            branch_id: request.branch_id.clone(),
            policy_id: request.policy.policy_id.clone(),
            policy_version: request.policy.version,
            cutoff: request.cutoff,
            corpus_generation: request.corpus_generation,
            revocation_epoch: self.revocation_epoch,
            selected_sources: kept,
            pinned_entry_ids: request.pinned_entry_ids.clone(),
            protected_manifest_sha256: request.mandatory_manifest_sha256.clone(),
            optional_byte_budget: request.policy.optional_byte_budget,
            optional_rendered_bytes: optional_bytes.len(),
            whole_rendered_bytes: rendered.len(),
            prepared_manifest_sha256: prepared_digest,
            whole_tokens: None,
            token_measurement: "unavailable".to_owned(),
            budget_status: "bounded_unknown_total".to_owned(),
            rendered_content_ref: request.prepared_content_ref.clone(),
            expires_at: request.expires_at.clone(),
            phase2_revision_id: request
                .policy
                .phase2_revision_id
                .clone()
                .ok_or(MemoryError::StaleApproval)?,
            effect_class: "local_preparation_only".to_owned(),
            phase2_prepared_manifest_sha256: request.phase2_prepared_manifest_sha256.clone(),
            rendered_bytes: rendered,
            exclusions,
        })
    }
}
