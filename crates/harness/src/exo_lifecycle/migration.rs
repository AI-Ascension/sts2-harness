// SPDX-License-Identifier: MIT

use super::*;
use crate::provider_session::owner_journal::{JournalSnapshot, OwnerJournal, read_legacy};
use crate::provider_session::{
    NativeCapabilities, ProviderSessionMetadataStore, ProviderSessionPolicy,
};
use std::sync::Arc;

impl LifecycleOwner {
    /// The authenticated claim must prove legacy owner quiescence and permanent route cutover.
    /// The legacy bytes are never modified; the destination must not exist.
    #[allow(clippy::too_many_arguments)]
    pub fn import_legacy(
        config: JournalConfig,
        key: [u8; 32],
        owner_token: String,
        policy: &ProviderSessionPolicy,
        capabilities: &NativeCapabilities,
        source_digest: String,
        cutover_ref: String,
        authority: Arc<dyn LifecycleAuthorityPort>,
    ) -> Result<Self, LifecycleError> {
        config.validate()?;
        if !types::digest(&source_digest) || !types::id(&cutover_ref) {
            return Err(LifecycleError::Invalid);
        }
        let path = config.legacy_path.as_ref().ok_or(LifecycleError::Invalid)?;
        let kind = ClaimKind::ImportLegacy {
            source_digest: source_digest.clone(),
            cutover_ref: cutover_ref.clone(),
        };
        let guard = authority.claim(&OwnerClaim {
            config: &config,
            kind: &kind,
        })?;
        let bytes = read_legacy(path)?;
        if crate::sha256_hex(&bytes) != source_digest {
            return Err(LifecycleError::Stale);
        }
        let legacy = ProviderSessionMetadataStore::encrypted(path, key, config.scope.clone())
            .map_err(|_| LifecycleError::Corrupt)?;
        let broker = legacy
            .load_envelope(&bytes, owner_token.clone(), policy, capabilities)
            .map_err(|_| LifecycleError::Corrupt)?;
        let mut snapshot = JournalSnapshot::new(&config, broker.snapshot());
        snapshot.import_digest = Some(source_digest);
        snapshot.cutover_ref = Some(cutover_ref);
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
        })
    }
}
