// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCapabilities {
    pub schema: String,
    pub product_phase: u8,
    pub scope: MemoryScope,
    pub enabled: bool,
    pub local_lexical_retrieval: String,
    pub extractive_compaction: String,
    pub abstractive_adapter: String,
    pub abstractive_live_verified: bool,
    pub per_decision_policy: String,
    pub phase2_approval_required: bool,
    pub persistent_provider_sessions: bool,
    pub provider_side_compaction: bool,
    pub semantic_vector_retrieval: String,
    pub hidden_reasoning_access: bool,
    pub direct_game_dispatch: bool,
    /// Limits enforced by this owner for the advertised policy revision. These are deliberately
    /// separate from the broader JSON Schema ceilings, which only establish portable syntax.
    pub effective_limits: EffectiveMemoryLimits,
    pub supported_operations: Vec<String>,
}

/// Owner-enforced policy limits bound to a capability response.
///
/// A policy can be valid against the public schema while exceeding one of these values. Callers
/// must use this descriptor for admission and preserve an over-limit policy for inspection rather
/// than clamping it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveMemoryLimits {
    pub policy_schema: String,
    pub max_candidates: usize,
    pub max_results: usize,
    pub max_selected: usize,
    pub optional_byte_budget: usize,
    pub max_entries_per_run: usize,
    pub max_corpus_bytes: usize,
}

impl MemoryCorpus {
    pub fn capabilities(&self) -> MemoryCapabilities {
        MemoryCapabilities {
            schema: MEMORY_CAPABILITIES_SCHEMA.to_owned(),
            product_phase: 3,
            scope: self.scope.clone(),
            enabled: self.enabled,
            local_lexical_retrieval: "supported".to_owned(),
            extractive_compaction: "supported".to_owned(),
            abstractive_adapter: "supported".to_owned(),
            abstractive_live_verified: false,
            per_decision_policy: "supported".to_owned(),
            phase2_approval_required: true,
            persistent_provider_sessions: false,
            provider_side_compaction: false,
            semantic_vector_retrieval: "unsupported".to_owned(),
            hidden_reasoning_access: false,
            direct_game_dispatch: false,
            effective_limits: EffectiveMemoryLimits {
                policy_schema: MEMORY_POLICY_SCHEMA.to_owned(),
                max_candidates: MAX_CANDIDATES,
                max_results: MAX_RESULTS,
                max_selected: MAX_SELECTED,
                optional_byte_budget: MAX_OPTIONAL_BYTES,
                max_entries_per_run: MAX_ENTRIES_PER_RUN,
                max_corpus_bytes: MAX_CORPUS_BYTES,
            },
            supported_operations: if self.enabled {
                vec![
                    "search", "extract", "generate", "review", "select", "policy", "adopt",
                    "revoke", "evaluate",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect()
            } else {
                Vec::new()
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum MemoryRole {
    Read,
    Search,
    Generate,
    Review,
    Select,
    Policy,
    Revoke,
    Control,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryAuthorizer {
    grants: BTreeMap<String, BTreeSet<MemoryRole>>,
}

impl MemoryAuthorizer {
    pub fn new() -> Self {
        Self {
            grants: BTreeMap::new(),
        }
    }

    pub fn grant(
        &mut self,
        principal: impl Into<String>,
        role: MemoryRole,
    ) -> Result<(), MemoryError> {
        let principal = principal.into();
        if !valid_id(&principal) {
            return Err(MemoryError::PermissionDenied);
        }
        self.grants.entry(principal).or_default().insert(role);
        Ok(())
    }

    pub fn check(&self, principal: &str, role: MemoryRole) -> Result<(), MemoryError> {
        if self
            .grants
            .get(principal)
            .is_some_and(|roles| roles.contains(&role))
        {
            Ok(())
        } else {
            Err(MemoryError::PermissionDenied)
        }
    }
}

impl Default for MemoryAuthorizer {
    fn default() -> Self {
        Self::new()
    }
}
