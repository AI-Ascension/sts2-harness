// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;
use serde_json::json;

impl ProviderSessionBroker {
    pub(super) fn projected_history_bytes(
        &self,
        binding_id: &str,
        items: &[HistoryItem],
    ) -> Result<usize, SessionError> {
        let existing = self
            .histories
            .iter()
            .filter(|(current_id, _)| current_id.as_str() != binding_id)
            .try_fold(0_usize, |total, (_, current)| {
                let bytes = serde_json::to_vec(current).map_err(|_| SessionError::Protocol)?;
                total.checked_add(bytes.len()).ok_or(SessionError::Capacity)
            })?;
        let replacement = serde_json::to_vec(items).map_err(|_| SessionError::Protocol)?;
        existing
            .checked_add(replacement.len())
            .filter(|size| *size <= MAX_HISTORY_BYTES)
            .ok_or(SessionError::Capacity)
    }

    pub fn refresh_history(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        idempotency_key: &str,
        items: Vec<HistoryItem>,
        watermark: u64,
        complete_at_watermark: bool,
    ) -> Result<NativeOperation, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "thread/read")
        {
            return Err(SessionError::Unsupported);
        }
        let binding = self.ensure_binding_not_expired(binding_id)?;
        if matches!(binding.state, BindingState::Retired | BindingState::Closed)
            || items.len() > MAX_SESSION_ITEMS
            || items.iter().any(|item| item.validate().is_err())
            || items.iter().any(|item| item.sequence > watermark)
            || items
                .windows(2)
                .any(|pair| pair[0].sequence >= pair[1].sequence)
        {
            return Err(SessionError::InvalidRequest);
        }
        if binding.state == BindingState::Quarantined {
            return Err(SessionError::Fenced);
        }
        if binding.state == BindingState::Candidate {
            return Err(SessionError::HeldRequired);
        }
        if items
            .iter()
            .filter(|item| item.kind == HistoryItemKind::ValidatedDecision)
            .count()
            > self.policy.max_completed_turns
        {
            return Err(SessionError::Capacity);
        }
        if !valid_id(idempotency_key) {
            return Err(SessionError::InvalidRequest);
        }
        if complete_at_watermark
            && watermark > 0
            && items.last().is_none_or(|item| item.sequence != watermark)
        {
            return Err(SessionError::Stale);
        }
        self.projected_history_bytes(binding_id, &items)?;
        let request = json!({"binding_id": binding_id, "watermark": watermark, "items": items, "complete_at_watermark": complete_at_watermark});
        if let Some(existing) = self.existing_idempotent(idempotency_key, &request)? {
            return Ok(existing);
        }
        let operation = self.new_operation(
            binding_id,
            NativeOperationKind::Refresh,
            idempotency_key,
            &request,
            false,
        )?;
        let entry = self.histories.entry(binding_id.to_owned()).or_default();
        *entry = items;
        if let Some(binding) = self.bindings.get_mut(binding_id) {
            binding.history_epoch = binding.history_epoch.saturating_add(1);
            binding.history_coverage = if complete_at_watermark {
                HistoryCoverage::ApplicationManifestVerified
            } else {
                HistoryCoverage::ReportedPartial
            };
        }
        if let Some(operation_mut) = self.operations.get_mut(&operation.operation_id) {
            operation_mut.state = NativeOperationState::Completed;
            operation_mut.terminal_evidence_ref = Some(format!("history-{watermark}"));
        }
        self.idempotency.insert(
            idempotency_key.to_owned(),
            (
                operation.request_sha256.clone(),
                operation.operation_id.clone(),
            ),
        );
        Ok(operation)
    }

    pub fn history(
        &self,
        binding_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<HistoryView, SessionError> {
        let binding = self.ensure_binding_not_expired(binding_id)?;
        if limit == 0 || limit > 128 {
            return Err(SessionError::Capacity);
        }
        let start = match cursor {
            None => 0,
            Some(value) => {
                let (epoch, offset) = value.split_once(':').ok_or(SessionError::Stale)?;
                if epoch.parse::<u64>().ok() != Some(binding.history_epoch) {
                    return Err(SessionError::Stale);
                }
                offset.parse::<usize>().map_err(|_| SessionError::Stale)?
            }
        };
        let items = self.histories.get(binding_id).cloned().unwrap_or_default();
        if start > items.len() {
            return Err(SessionError::Stale);
        }
        let end = start.saturating_add(limit).min(items.len());
        let next_cursor = (end < items.len()).then(|| format!("{}:{end}", binding.history_epoch));
        let coverage = match binding.history_coverage {
            HistoryCoverage::ApplicationManifestVerified => {
                HistoryCoverageView::CompleteAtWatermark
            }
            HistoryCoverage::ReportedPartial => HistoryCoverageView::Partial,
            HistoryCoverage::Unknown => HistoryCoverageView::Unknown,
        };
        Ok(HistoryView {
            schema: SESSION_HISTORY_SCHEMA.to_owned(),
            view_id: format!("history-view-{}", binding_id),
            binding_id: binding_id.to_owned(),
            scope: self.scope.clone(),
            history_epoch: binding.history_epoch,
            watermark: items.last().map_or(0, |item| item.sequence),
            coverage,
            effective_context_coverage: "unknown",
            items: items[start..end].to_vec(),
            known_total_items: Some(items.len()),
            next_cursor,
            read_started_turn: false,
            expires_at: binding.expires_at.clone(),
        })
    }
}
