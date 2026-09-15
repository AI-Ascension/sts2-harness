// SPDX-License-Identifier: MIT
use super::*;
use crate::context_memory::MAX_SOURCE_BYTES;

pub(super) fn tool_name(request: &Value) -> Result<&'static str, LookupError> {
    match request["query"]["query_kind"].as_str() {
        Some("list") => Ok("sts2.game_information_list"),
        Some("search") => Ok("sts2.game_information_search"),
        Some("get") => Ok("sts2.game_information_get"),
        Some("detail") => Ok("sts2.game_information_detail"),
        Some("availability") => Ok("sts2.game_information_availability"),
        _ => Err(LookupError::MissingCapability),
    }
}

impl LookupSession {
    pub(super) fn admit(
        &self,
        binding: &LookupBinding,
        operation: &str,
        request: &Value,
        corpus: &MemoryCorpus,
    ) -> Result<(), LookupError> {
        self.policy
            .validate(corpus)
            .map_err(|_| LookupError::Scope)?;
        if binding != &self.binding
            || operation.is_empty()
            || operation.len() > 128
            || !operation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        {
            return Err(LookupError::Scope);
        }
        if self.records.len() >= 256 {
            return Err(LookupError::Bounds);
        }
        validation::validate_request(request)?;
        let query = &request["query"];
        if query["binding"]["content_manifest_id"] != self.binding.content_manifest_id
            || query["binding"]["locale"] != self.binding.locale
        {
            return Err(LookupError::Scope);
        }
        if query["binding"]["mode"] == "live" {
            let snapshot = self
                .binding
                .snapshot
                .as_ref()
                .ok_or(LookupError::Reobserve)?;
            if query["binding"]["snapshot_ref"] != *snapshot
                || query["binding"]["instance_ref"]["run_id"] != self.binding.scope.run_id
            {
                return Err(LookupError::Reobserve);
            }
        }
        self.admit_capabilities(request)
    }

    fn admit_capabilities(&self, request: &Value) -> Result<(), LookupError> {
        let caps = self
            .capabilities
            .as_ref()
            .ok_or(LookupError::MissingCapability)?;
        let query = &request["query"];
        for (singular, plural) in [
            ("query_kind", "query_kinds"),
            ("entity_kind", "entity_kinds"),
            ("projection", "projections"),
            ("detail_level", "detail_levels"),
        ] {
            if !caps[plural]
                .as_array()
                .is_some_and(|items| items.contains(&query[singular]))
            {
                return Err(LookupError::MissingCapability);
            }
        }
        let fields = query["fields"].as_array().ok_or(LookupError::Invalid)?;
        if fields
            .iter()
            .any(|field| !caps["fields"].as_array().is_some_and(|v| v.contains(field)))
        {
            return Err(LookupError::MissingCapability);
        }
        for key in ["page_items", "item_bytes", "page_bytes", "text_bytes"] {
            if query["limits"][key].as_u64() > caps["limits"][key].as_u64() {
                return Err(LookupError::Bounds);
            }
        }
        let bytes = serde_json::to_vec(request).map_err(|_| LookupError::Invalid)?;
        if bytes.len() > MAX_SOURCE_BYTES
            || bytes.len() as u64
                > caps["max_message_bytes"]
                    .as_u64()
                    .ok_or(LookupError::Invalid)?
            || query["cursor"]
                .as_str()
                .is_some_and(|c| c.len() as u64 > caps["max_cursor_bytes"].as_u64().unwrap_or(0))
        {
            return Err(LookupError::Bounds);
        }
        Ok(())
    }

    pub(super) fn page_sequence(
        &self,
        operation: &str,
        request: &Value,
    ) -> Result<usize, LookupError> {
        let mut query = request["query"].clone();
        let cursor = query["cursor"].take();
        match self.pages.get(operation) {
            None if cursor.is_null() => Ok(0),
            Some((first, next, count))
                if first == &query && next == &cursor && !next.is_null() && *count < MAX_PAGES =>
            {
                Ok(*count)
            }
            _ => Err(LookupError::Divergence),
        }
    }
}
