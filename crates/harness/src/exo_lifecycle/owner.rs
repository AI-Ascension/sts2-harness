// SPDX-License-Identifier: MIT

use super::*;
use crate::provider_session::owner_journal::{JournalSnapshot, OwnerJournal};
use crate::provider_session::{NativeCapabilities, ProviderSessionBroker, ProviderSessionPolicy};
use std::sync::Arc;

use super::derived_ids::{
    derived_binding_id, derived_prepared_id, derived_provider_attempt_id, identity_token,
};

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
    /// Uses this owner's broker to persist the one-shot binding, exact prepared input and native
    /// operation before `start` can create a process.  The returned manifest is deliberately
    /// completed from broker-assigned identities; callers never predict a native operation id.
    pub fn prepare_one_shot_manifest(
        &mut self,
        mut manifest: InvocationManifest,
        input: &[u8],
        expires_at: &str,
    ) -> Result<InvocationManifest, LifecycleError> {
        self.check()?;
        if input.len() != manifest.input_length || crate::sha256_hex(input) != manifest.input_digest
        {
            return Err(LifecycleError::Invalid);
        }
        if let Some(completed) = self.completed_manifest_for_request(&manifest, input)? {
            return Ok(completed);
        }
        // The request-level identity is admitted up to the published 512-byte wire width, but
        // every internal identity the manifest (`id`) and the broker (`provider_session::valid_id`)
        // validate is capped at 128. Identifiers this owner mints from the identity therefore
        // carry a digest of it instead of the identity itself, so their width is a function of
        // the prefix alone and no admitted identity has a hidden ceiling (ADR 0077).
        let selection_id = identity_token(&manifest.execution_id);
        let binding_id = derived_binding_id(&manifest.execution_id);
        let prepared_id = derived_prepared_id(&manifest.execution_id);
        if self.broker.binding(&binding_id).is_err() {
            self.broker
                .admit_one_shot_binding(&self.token, &binding_id, expires_at)
                .map_err(|_| LifecycleError::Held)?;
        }
        let binding = self
            .broker
            .binding(&binding_id)
            .map_err(|_| LifecycleError::Held)?
            .clone();
        let prepared = match self.broker.prepare_turn(
            &self.token,
            &binding_id,
            &prepared_id,
            &manifest.request_id,
            &manifest.host_turn_id,
            &selection_id,
            &manifest.host_turn_id,
            input.to_vec(),
            br#"{"type":"object"}"#.to_vec(),
            Vec::new(),
            Vec::new(),
            expires_at,
        ) {
            Ok(prepared) => prepared,
            Err(crate::provider_session::SessionError::Conflict) => self
                .broker
                .prepared(&prepared_id)
                .map_err(|_| LifecycleError::Held)?
                .clone(),
            Err(_) => return Err(LifecycleError::Held),
        };
        let operation = self
            .broker
            .admit_turn(
                &self.token,
                &binding_id,
                &prepared.prepared_id,
                &selection_id,
            )
            .map_err(|_| LifecycleError::Held)?;
        manifest.binding_id = binding.binding_id;
        manifest.prepared_id = prepared.prepared_id;
        manifest.operation_id = operation.operation_id;
        manifest.reservation_id = format!("provider-reservation-{}", manifest.operation_id);
        manifest.provider_attempt_id = derived_provider_attempt_id(&manifest.execution_id);
        manifest.authority.owner_epoch = self.broker.owner_epoch();
        manifest.authority.auth_epoch = self.broker.owner_epoch();
        manifest.authority.session_epoch = binding.session_epoch;
        manifest.authority.history_epoch = binding.history_epoch;
        manifest.authority.compaction_epoch = binding.compaction_epoch;
        manifest.authority.revocation_epoch = self.broker.revocation_epoch();
        Ok(manifest)
    }

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
