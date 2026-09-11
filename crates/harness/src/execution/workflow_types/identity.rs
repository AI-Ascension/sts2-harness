// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::types::{ExecutionStoreError, valid_id, valid_reference};

pub const WORKFLOW_CONTRACT_VERSION: &str = "workflow-v1";
pub const MAX_WORKFLOW_BYTES: usize = 1024 * 1024;
pub const MAX_WORKFLOW_STACK_DEPTH: usize = 128;
pub const MAX_WORKFLOW_COUNTERS: usize = 128;
pub const MAX_WORKFLOW_COUNTER_NAME_BYTES: usize = 128;
pub const MAX_WORKFLOW_CURSOR: u64 = 9_007_199_254_740_991;

macro_rules! namespaced_id {
    ($name:ident, $namespace:literal) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub const NAMESPACE: &'static str = $namespace;

            pub fn new(value: impl Into<String>) -> Result<Self, ExecutionStoreError> {
                let value = value.into();
                if !valid_id(&value) {
                    return Err(ExecutionStoreError::InvalidWorkflowIdentity);
                }
                Ok(Self(value))
            }

            pub(crate) fn validate(&self) -> Result<(), ExecutionStoreError> {
                if valid_id(&self.0) {
                    Ok(())
                } else {
                    Err(ExecutionStoreError::InvalidWorkflowIdentity)
                }
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

namespaced_id!(WorkflowDefinitionId, "workflow");
namespaced_id!(WorkflowPlanId, "workflow.plan");
namespaced_id!(WorkflowRunId, "workflow.run");
namespaced_id!(WorkflowEpisodeId, "workflow.episode");
namespaced_id!(WorkflowCommandId, "workflow.command");
namespaced_id!(GameOperationId, "game.operation");
namespaced_id!(WorkflowInvocationId, "workflow.invocation");
namespaced_id!(WorkflowEventId, "workflow.event");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowDefinition {
    pub id: WorkflowDefinitionId,
    pub bytes: Vec<u8>,
    pub digest: String,
}

impl WorkflowDefinition {
    pub fn new(id: WorkflowDefinitionId, bytes: Vec<u8>) -> Result<Self, ExecutionStoreError> {
        id.validate()?;
        validate_bytes(&bytes)?;
        Ok(Self {
            id,
            digest: digest_bytes(&bytes),
            bytes,
        })
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        self.id.validate()?;
        validate_bytes(&self.bytes)?;
        if self.digest != digest_bytes(&self.bytes) {
            return Err(ExecutionStoreError::InvalidWorkflowRecord);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowPlan {
    pub id: WorkflowPlanId,
    pub workflow_id: WorkflowDefinitionId,
    pub bytes: Vec<u8>,
    pub digest: String,
}

impl WorkflowPlan {
    pub fn new(
        id: WorkflowPlanId,
        workflow_id: WorkflowDefinitionId,
        bytes: Vec<u8>,
    ) -> Result<Self, ExecutionStoreError> {
        id.validate()?;
        workflow_id.validate()?;
        validate_bytes(&bytes)?;
        Ok(Self {
            id,
            workflow_id,
            digest: digest_bytes(&bytes),
            bytes,
        })
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        self.id.validate()?;
        self.workflow_id.validate()?;
        validate_bytes(&self.bytes)?;
        if self.digest != digest_bytes(&self.bytes) {
            return Err(ExecutionStoreError::InvalidWorkflowRecord);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RunStatus {
    Created,
    Running,
    Completed,
    Failed,
}

impl RunStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "created" => Self::Created,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunProjection {
    pub status: RunStatus,
    pub cursor: u64,
    pub stack: Vec<String>,
    pub counters: BTreeMap<String, u64>,
}

impl RunProjection {
    pub fn new() -> Self {
        Self {
            status: RunStatus::Created,
            cursor: 0,
            stack: Vec::new(),
            counters: BTreeMap::new(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ExecutionStoreError> {
        if self.cursor > MAX_WORKFLOW_CURSOR
            || self.stack.len() > MAX_WORKFLOW_STACK_DEPTH
            || self.counters.len() > MAX_WORKFLOW_COUNTERS
            || self.stack.iter().any(|value| !valid_reference(value))
            || self.counters.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > MAX_WORKFLOW_COUNTER_NAME_BYTES
                    || !valid_reference(key)
                    || *value > 9_007_199_254_740_991
            })
        {
            return Err(ExecutionStoreError::InvalidWorkflowProjection);
        }
        Ok(())
    }

    pub(crate) fn increment(&mut self, name: &str) -> Result<(), ExecutionStoreError> {
        if name.is_empty() || name.len() > MAX_WORKFLOW_COUNTER_NAME_BYTES {
            return Err(ExecutionStoreError::InvalidWorkflowProjection);
        }
        let value = self.counters.entry(String::from(name)).or_insert(0);
        *value = value
            .checked_add(1)
            .ok_or(ExecutionStoreError::InvalidWorkflowProjection)?;
        self.validate()
    }
}

impl Default for RunProjection {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(crate) fn validate_bytes(bytes: &[u8]) -> Result<(), ExecutionStoreError> {
    if bytes.is_empty() || bytes.len() > MAX_WORKFLOW_BYTES {
        return Err(ExecutionStoreError::InvalidWorkflowRecord);
    }
    Ok(())
}

const HEX: &[u8; 16] = b"0123456789abcdef";
