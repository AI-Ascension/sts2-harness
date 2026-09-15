// SPDX-License-Identifier: MIT

use super::*;
use crate::provider_session::owner_journal::{JournalSnapshot, OwnerJournal};
use crate::provider_session::{NativeCapabilities, ProviderSessionBroker, ProviderSessionPolicy};
use std::sync::Arc;

pub struct LifecycleOwner {
    pub(super) broker: ProviderSessionBroker,
    pub(super) token: zeroize::Zeroizing<String>,
    pub(super) journal: OwnerJournal,
    pub(super) snapshot: JournalSnapshot,
    pub(super) authority: Arc<dyn LifecycleAuthorityPort>,
    pub(super) poisoned: bool,
    pub(super) instance: Arc<()>,
    pub(super) store_instance: Option<Arc<()>>,
}

/// This handle cannot mint another send permit. Polling never holds an authority guard.
pub struct InFlight<H> {
    pub(super) handle: H,
    pub(super) manifest: InvocationManifest,
    pub(super) owner_epoch: u64,
    pub(super) input: Vec<u8>,
    pub(super) settled: bool,
    pub(super) instance: Arc<()>,
    pub(super) store_instance: Arc<()>,
}

pub enum StartOutcome<H> {
    Started(Box<InFlight<H>>),
    Stored(crate::BoundDecision),
}

impl LifecycleOwner {
    pub fn create(
        config: JournalConfig,
        key: [u8; 32],
        broker: ProviderSessionBroker,
        owner_token: String,
        authority: Arc<dyn LifecycleAuthorityPort>,
    ) -> Result<Self, LifecycleError> {
        config.validate()?;
        broker
            .authorize_owner(&owner_token)
            .map_err(|_| LifecycleError::Fenced)?;
        let snapshot = JournalSnapshot::new(&config, broker.snapshot());
        snapshot.validate(&config)?;
        let guard = authority.claim(&OwnerClaim {
            config: &config,
            kind: &ClaimKind::Create,
        })?;
        let journal = OwnerJournal::create(config.clone(), key, &snapshot)?;
        drop(guard);
        Ok(Self {
            broker,
            token: zeroize::Zeroizing::new(owner_token),
            journal,
            snapshot,
            authority,
            poisoned: false,
            instance: Arc::new(()),
            store_instance: None,
        })
    }

    pub fn open(
        config: JournalConfig,
        key: [u8; 32],
        owner_token: String,
        policy: &ProviderSessionPolicy,
        capabilities: &NativeCapabilities,
        authority: Arc<dyn LifecycleAuthorityPort>,
    ) -> Result<Self, LifecycleError> {
        config.validate()?;
        let (mut journal, mut snapshot) = OwnerJournal::open(config.clone(), key)?;
        let bytes = serde_json::to_vec(&snapshot.broker).map_err(|_| LifecycleError::Corrupt)?;
        let broker = ProviderSessionBroker::from_snapshot_json_checked(
            &bytes,
            owner_token.clone(),
            &config.scope,
            policy,
            capabilities,
        )
        .map_err(|_| LifecycleError::Corrupt)?;
        let guard = authority.claim(&OwnerClaim {
            config: &config,
            kind: &ClaimKind::Restart,
        })?;
        snapshot.broker = broker.snapshot();
        snapshot.claim_epoch = snapshot.broker.owner_epoch;
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or(LifecycleError::Capacity)?;
        for entry in &mut snapshot.entries {
            if entry.phase == LifecyclePhase::Sent {
                entry.phase = LifecyclePhase::Unknown;
            }
        }
        journal.commit(&snapshot)?;
        drop(guard);
        Ok(Self {
            broker,
            token: zeroize::Zeroizing::new(owner_token),
            journal,
            snapshot,
            authority,
            poisoned: false,
            instance: Arc::new(()),
            store_instance: None,
        })
    }

    pub fn entries(&self) -> &[LifecycleEntry] {
        &self.snapshot.entries
    }

    pub fn claim_epoch(&self) -> u64 {
        self.snapshot.claim_epoch
    }

    pub(super) fn check(&mut self) -> Result<(), LifecycleError> {
        if self.poisoned {
            return Err(LifecycleError::Poisoned);
        }
        if let Err(error) = self.journal.check() {
            self.poisoned = true;
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn persist(&mut self) -> Result<(), LifecycleError> {
        self.snapshot.broker = self.broker.snapshot();
        let Some(revision) = self.snapshot.revision.checked_add(1) else {
            self.poisoned = true;
            return Err(LifecycleError::Capacity);
        };
        self.snapshot.revision = revision;
        if let Err(error) = self.journal.commit(&self.snapshot) {
            self.poisoned = true;
            return Err(error);
        }
        Ok(())
    }
}
