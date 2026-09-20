// SPDX-License-Identifier: MIT
use super::*;
use crate::context_memory::{EvidenceStatus, MAX_SOURCE_BYTES, MemoryEntry, MemoryKind};
use serde_json::json;

pub(super) fn retained<'a>(
    record: &LookupRecord,
    corpus: &'a MemoryCorpus,
    now: &str,
) -> Result<&'a [u8], LookupError> {
    let reference = record
        .source
        .as_ref()
        .ok_or(LookupError::MissingRetention)?;
    let entry = corpus
        .entry(reference)
        .ok_or(LookupError::MissingRetention)?;
    if entry.scope != record.binding.scope || entry.game_profile != record.binding.game_profile {
        return Err(LookupError::Scope);
    }
    let bytes = corpus
        .read_content(
            reference,
            "game-information",
            9_007_199_254_740_991,
            corpus.generation(),
            now,
        )
        .map_err(|_| LookupError::MissingRetention)?;
    if Some(crate::sha256_hex(bytes)) != record.source_sha256 || bytes.len() != record.source_bytes
    {
        return Err(LookupError::Divergence);
    }
    Ok(bytes)
}

impl LookupSession {
    /// Bounded exact raw-byte chunk of a retained, previously admitted source. Chunks are data.
    pub fn read_retained(
        &self,
        record: &LookupRecord,
        corpus: &MemoryCorpus,
        offset: usize,
    ) -> Result<Vec<u8>, LookupError> {
        if !record.binding.same_owner(&self.binding) || corpus.scope() != &self.binding.scope {
            return Err(LookupError::Scope);
        }
        self.replay(record, &record.request, corpus)?;
        let bytes = retained(record, corpus, &self.now)?;
        if offset > bytes.len() {
            return Err(LookupError::Bounds);
        }
        let end = offset
            .saturating_add(self.policy.optional_byte_budget)
            .min(bytes.len());
        Ok(bytes[offset..end].to_vec())
    }

    fn new_record(&self, operation: &str, sequence: usize, request: Value) -> LookupRecord {
        LookupRecord {
            schema: "ascension.game-information-record.v1".to_owned(),
            binding: self.binding.clone(),
            operation_id: operation.to_owned(),
            page_sequence: sequence,
            request,
            source_sha256: None,
            source: None,
            source_bytes: 0,
            view_sha256: None,
            error: None,
        }
    }

    pub(super) fn record_error(
        &mut self,
        operation: &str,
        sequence: usize,
        request: Value,
        error: LookupError,
    ) {
        let mut record = self.new_record(operation, sequence, request);
        record.error = Some(error);
        self.records.push(record);
    }

    pub(super) fn accept(
        &mut self,
        operation: &str,
        sequence: usize,
        request: Value,
        raw: Vec<u8>,
        corpus: &mut MemoryCorpus,
    ) -> Result<LookupDelivery, LookupError> {
        let outcome = self.accept_source(operation, sequence, &request, &raw, corpus);
        match outcome {
            Ok(delivery) => {
                let value = validation::decode_strict(&raw)?;
                let mut query = request["query"].clone();
                query["cursor"] = Value::Null;
                self.pages.insert(
                    operation.to_owned(),
                    (
                        query,
                        value["result"]["page"]["next_cursor"].clone(),
                        sequence + 1,
                    ),
                );
                self.records.push(delivery.record.clone());
                Ok(delivery)
            }
            Err(error) => {
                self.record_error(operation, sequence, request, error.clone());
                Err(error)
            }
        }
    }

    fn accept_source(
        &self,
        operation: &str,
        sequence: usize,
        request: &Value,
        raw: &[u8],
        corpus: &mut MemoryCorpus,
    ) -> Result<LookupDelivery, LookupError> {
        let caps = self
            .capabilities
            .as_ref()
            .ok_or(LookupError::MissingCapability)?;
        if raw.len() > MAX_SOURCE_BYTES
            || raw.len() as u64
                > caps["max_message_bytes"]
                    .as_u64()
                    .ok_or(LookupError::Invalid)?
        {
            return Err(LookupError::Bounds);
        }
        let value = validation::decode_strict(raw)?;
        validation::validate_response(request, &value)?;
        if value["result"]["page"]["next_cursor"]
            .as_str()
            .is_some_and(|cursor| {
                cursor.len() as u64 > caps["max_cursor_bytes"].as_u64().unwrap_or(0)
            })
        {
            return Err(LookupError::Bounds);
        }
        if value["kind"] == "error_response" {
            return Err(match value["error"]["code"].as_str() {
                Some("stale_snapshot" | "stale_cursor" | "mixed_generation") => {
                    LookupError::Reobserve
                }
                Some(_) => LookupError::Producer(
                    serde_json::from_value(value["error"]["code"].clone())
                        .map_err(|_| LookupError::Invalid)?,
                ),
                None => LookupError::Invalid,
            });
        }
        for item in value["result"]["page"]["items"]
            .as_array()
            .ok_or(LookupError::Invalid)?
        {
            for field in item["fields"].as_array().ok_or(LookupError::Invalid)? {
                if !caps["fields"]
                    .as_array()
                    .is_some_and(|fields| fields.contains(&field["name"]))
                {
                    return Err(LookupError::MissingCapability);
                }
            }
        }
        self.validate_page_chain(operation, &value, corpus)?;
        let mut record = self.new_record(operation, sequence, request.clone());
        let entry_id = format!(
            "lookup:{}",
            crate::sha256_hex(serde_json::to_vec(&record).map_err(|_| LookupError::Invalid)?)
        );
        let entry = MemoryEntry::new(
            self.binding.scope.clone(),
            &entry_id,
            operation,
            if request["query"]["binding"]["mode"] == "static" {
                MemoryKind::StaticReference
            } else {
                MemoryKind::HistoricalObservation
            },
            EvidenceStatus::Reported,
            "game-information",
            &entry_id,
            raw.to_vec(),
            sequence as u64,
            sequence as u64,
            corpus.generation() + 1,
            &self.now,
            &self.expires_at,
            &self.binding.game_profile,
            false,
        );
        record.source = Some(entry.reference());
        record.source_bytes = raw.len();
        record.source_sha256 = Some(crate::sha256_hex(raw));
        corpus.admit(entry).map_err(|_| LookupError::Retention)?;
        self.deliver(record, &value)
    }

    pub(super) fn deliver(
        &self,
        mut record: LookupRecord,
        value: &Value,
    ) -> Result<LookupDelivery, LookupError> {
        let full = json!({"authority":"untrusted_game_information_data","protocol_version":PROFILE,"schema_digest":SCHEMA_DIGEST,"source_sha256":record.source_sha256,
            "binding":record.binding,"result":value["result"]});
        let full_bytes = serde_json::to_vec(&full).map_err(|_| LookupError::Invalid)?;
        let data = if full_bytes.len() <= self.policy.optional_byte_budget {
            full
        } else {
            json!({"authority":"untrusted_game_information_data","protocol_version":PROFILE,"schema_digest":SCHEMA_DIGEST,"retained_source":record.source,
                "source_sha256":record.source_sha256,"byte_length":record.source_bytes,"delivery":"retained"})
        };
        let encoded = serde_json::to_vec(&data).map_err(|_| LookupError::Invalid)?;
        if encoded.len() > self.policy.optional_byte_budget {
            return Err(LookupError::Bounds);
        }
        record.view_sha256 = Some(crate::sha256_hex(encoded));
        Ok(LookupDelivery { record, data })
    }

    fn validate_page_chain(
        &self,
        operation: &str,
        value: &Value,
        corpus: &MemoryCorpus,
    ) -> Result<(), LookupError> {
        let page = &value["result"]["page"];
        if !page["next_cursor"].is_null() && page["next_cursor"] == value["query"]["cursor"] {
            return Err(LookupError::Divergence);
        }
        let mut items = Vec::new();
        for previous in self
            .records
            .iter()
            .filter(|record| record.operation_id == operation && record.source.is_some())
        {
            let source = validation::decode_strict(retained(previous, corpus, &self.now)?)?;
            let previous_page = &source["result"]["page"];
            for key in ["ordering", "total_count", "total_count_known", "coverage"] {
                if previous_page[key] != page[key] {
                    return Err(LookupError::Divergence);
                }
            }
            if !page["next_cursor"].is_null()
                && previous.request["query"]["cursor"] == page["next_cursor"]
            {
                return Err(LookupError::Divergence);
            }
            items.extend(
                previous_page["items"]
                    .as_array()
                    .ok_or(LookupError::Invalid)?
                    .iter()
                    .cloned(),
            );
        }
        items.extend(
            page["items"]
                .as_array()
                .ok_or(LookupError::Invalid)?
                .iter()
                .cloned(),
        );
        let mut identities = std::collections::BTreeSet::new();
        for item in &items {
            let key = if item["instance_ref"].is_null() {
                &item["definition_ref"]
            } else {
                &item["instance_ref"]
            };
            if !identities.insert(serde_json::to_vec(key).map_err(|_| LookupError::Invalid)?) {
                return Err(LookupError::Divergence);
            }
        }
        validation::validate_item_order(&items, &page["ordering"])?;
        if page["final_page"] == true
            && page["coverage"] == "complete"
            && page["total_count_known"] == true
            && page["total_count"].as_u64() != Some(items.len() as u64)
        {
            return Err(LookupError::Divergence);
        }
        Ok(())
    }
}
