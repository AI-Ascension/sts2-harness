// SPDX-License-Identifier: MIT

use crate::exo_lifecycle::{JournalConfig, LifecycleEntry, LifecycleError, MAX_LIFECYCLE_ENTRIES};
use crate::provider_session::{BrokerSnapshot, MAX_HISTORY_BYTES, SessionScope};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(crate) const MAGIC: &[u8] = b"ASCENSION-PROVIDER-METADATA-ENC2\0";
pub(crate) const MAX_ENVELOPE: usize = MAX_HISTORY_BYTES + MAGIC.len() + 24 + 16;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalSnapshot {
    pub schema: String,
    pub store_id: String,
    pub scope: SessionScope,
    pub owner_binding_digest: String,
    pub claim_epoch: u64,
    pub revision: u64,
    pub broker: BrokerSnapshot,
    pub entries: Vec<LifecycleEntry>,
    pub import_digest: Option<String>,
    pub cutover_ref: Option<String>,
}

impl JournalSnapshot {
    pub fn new(config: &JournalConfig, broker: BrokerSnapshot) -> Self {
        Self {
            schema: "ascension.provider-session.owner-journal.v2".into(),
            store_id: config.store_id.clone(),
            scope: config.scope.clone(),
            owner_binding_digest: config.owner_binding_digest.clone(),
            claim_epoch: broker.owner_epoch,
            revision: 1,
            broker,
            entries: Vec::new(),
            import_digest: None,
            cutover_ref: None,
        }
    }

    pub fn validate(&self, config: &JournalConfig) -> Result<(), LifecycleError> {
        let maximum = self
            .broker
            .policy
            .max_completed_turns
            .min(MAX_LIFECYCLE_ENTRIES);
        if self.schema != "ascension.provider-session.owner-journal.v2"
            || self.store_id != config.store_id
            || self.scope != config.scope
            || self.owner_binding_digest != config.owner_binding_digest
            || self.broker.scope != self.scope
            || self.broker.owner_epoch != self.claim_epoch
            || self.claim_epoch == 0
            || self.revision == 0
            || self.entries.len() > maximum
            || self.import_digest.is_some() != self.cutover_ref.is_some()
            || self
                .import_digest
                .as_ref()
                .is_some_and(|v| !crate::exo_lifecycle::types::digest(v))
            || self
                .cutover_ref
                .as_ref()
                .is_some_and(|v| !crate::exo_lifecycle::types::id(v))
        {
            return Err(LifecycleError::Corrupt);
        }
        let mut executions = BTreeSet::new();
        let mut operations = BTreeSet::new();
        let mut reservations = BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if entry.manifest.scope != self.scope
                || entry.claim_epoch > self.claim_epoch
                || entry
                    .permit_revision
                    .is_some_and(|revision| revision > self.revision)
                || !executions.insert(&entry.manifest.execution_id)
                || !operations.insert(&entry.manifest.operation_id)
                || !reservations.insert(&entry.manifest.reservation_id)
            {
                return Err(LifecycleError::Corrupt);
            }
        }
        Ok(())
    }
}
