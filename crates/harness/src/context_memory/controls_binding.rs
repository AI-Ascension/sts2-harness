// SPDX-License-Identifier: MIT

pub const MEMORY_BINDING_SCHEMA: &str = "ascension.context-memory.binding.v1";
pub const MAX_MEMORY_BINDINGS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingFailpoint {
    BeforeCommit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBinding {
    pub schema: String,
    pub binding_id: String,
    pub phase2_revision_id: String,
    pub phase2_preview_id: String,
    pub policy_id: String,
    pub policy_version: u64,
    pub selection_sha256: String,
    pub audit_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtomicBindingStore {
    bindings: BTreeMap<String, MemoryBinding>,
    failpoint: Option<BindingFailpoint>,
}

impl AtomicBindingStore {
    pub fn new() -> Self {
        Self {
            bindings: BTreeMap::new(),
            failpoint: None,
        }
    }

    pub fn set_failpoint(&mut self, failpoint: Option<BindingFailpoint>) {
        self.failpoint = failpoint;
    }

    pub fn commit(&mut self, binding: MemoryBinding) -> Result<MemoryBinding, MemoryError> {
        if binding.schema != MEMORY_BINDING_SCHEMA
            || !valid_id(&binding.binding_id)
            || !valid_id(&binding.phase2_revision_id)
            || !valid_id(&binding.phase2_preview_id)
            || !valid_id(&binding.policy_id)
            || binding.policy_version == 0
            || !valid_digest(&binding.selection_sha256)
            || !valid_digest(&binding.audit_sha256)
        {
            return Err(MemoryError::InvalidQuery);
        }
        if let Some(existing) = self.bindings.get(&binding.binding_id) {
            return if existing == &binding {
                Ok(existing.clone())
            } else {
                Err(MemoryError::Conflict)
            };
        }
        if self.failpoint.take().is_some() {
            return Err(MemoryError::PublicationFailed);
        }
        if self.bindings.len() >= MAX_MEMORY_BINDINGS {
            return Err(MemoryError::Capacity);
        }
        self.bindings
            .insert(binding.binding_id.clone(), binding.clone());
        Ok(binding)
    }

    pub fn binding(&self, binding_id: &str) -> Option<&MemoryBinding> {
        self.bindings.get(binding_id)
    }
}

impl Default for AtomicBindingStore {
    fn default() -> Self {
        Self::new()
    }
}
