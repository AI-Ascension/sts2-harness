// SPDX-License-Identifier: MIT

//! Authenticated, transport-free durable memory-policy ownership.
//!
//! This opt-in component prepares memory selections; it never invokes a provider or resumes
//! gameplay. Construction and authority updates belong to trusted composition, not command data.

#[path = "policy_owner_adoption.rs"]
mod adoption;
#[path = "policy_owner_authority.rs"]
mod authority;
#[path = "policy_owner_commands.rs"]
mod commands;
#[path = "policy_owner_integrity.rs"]
mod integrity;
#[path = "policy_owner_original.rs"]
mod original;
#[path = "policy_owner_prepare.rs"]
mod prepare;
#[path = "policy_owner_receipt_integrity.rs"]
mod receipt_integrity;
#[path = "policy_owner_records.rs"]
mod records;
#[path = "policy_store.rs"]
mod store;
#[path = "policy_store_schema.rs"]
mod store_schema;
#[path = "policy_owner_types.rs"]
mod types;

pub use authority::{MemoryPolicyAuthority, PolicyClock, TrustedPolicyState};
pub use prepare::{ActivePolicyPreparation, PreparedActivePolicy};
pub use records::{
    ActivePolicyBinding, PolicyApproval, PolicyFence, PolicyReceipt, PolicyReview, ReviewKind,
    SavedPolicy, SavedPolicyRef,
};
pub use store::{PolicyStoreConsent, PolicyStoreFailpoint};
pub use types::*;

use crate::context_memory::*;
use records::PolicyJournal;
use std::path::Path;
use std::sync::{Arc, Mutex};
use store::PolicyStore;

/// An owner-scoped entrypoint. Callers cannot supply the execution policy to preparation.
pub struct MemoryPolicyOwner {
    authority: Arc<MemoryPolicyAuthority>,
    store: Mutex<PolicyStore>,
}

impl MemoryPolicyOwner {
    pub fn open(
        path: impl AsRef<Path>,
        key: [u8; 32],
        authority: Arc<MemoryPolicyAuthority>,
        consent: PolicyStoreConsent,
    ) -> Result<Self, PolicyOwnerError> {
        let store = authority.inspect(|state| {
            PolicyStore::open(path.as_ref(), key, state.corpus.scope().clone(), consent)
        })?;
        Ok(Self {
            authority,
            store: Mutex::new(store),
        })
    }

    /// Only trusted test composition can inject a persistence failure.
    pub fn set_failpoint(
        &self,
        point: Option<PolicyStoreFailpoint>,
    ) -> Result<(), PolicyOwnerError> {
        self.store
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?
            .failpoint = point;
        Ok(())
    }

    pub fn execute(
        &self,
        access: PolicyAccess<'_>,
        command: PolicyCommand,
    ) -> Result<PolicyReceipt, PolicyOwnerError> {
        command.validate()?;
        self.authority
            .with_authorized(access, command.permission(), |state, actor, clock| {
                let mut store = self
                    .store
                    .lock()
                    .map_err(|_| PolicyOwnerError::Unavailable)?;
                store.change(
                    |journal| self.execute_once(journal, state, actor, clock, &command),
                    || state.check_actor(actor, command.permission(), clock.now_seconds()),
                )
            })
    }

    fn execute_once(
        &self,
        journal: &mut PolicyJournal,
        state: &TrustedPolicyState,
        actor: &authority::AuthorizedActor,
        clock: &dyn PolicyClock,
        command: &PolicyCommand,
    ) -> Result<PolicyReceipt, PolicyOwnerError> {
        let request_sha256 = command.fingerprint()?;
        if let Some(receipt) = journal.receipts.iter().find(|receipt| {
            receipt.subject == actor.subject && receipt.idempotency_key == command.key()
        }) {
            return if receipt.request_sha256 == request_sha256 {
                Ok(receipt.clone())
            } else {
                Err(PolicyOwnerError::Conflict)
            };
        }
        let outcome = self.apply(journal, state, actor, clock, command)?;
        state.check_actor(actor, command.permission(), clock.now_seconds())?;
        let receipt = PolicyReceipt {
            operation_id: format!("policy-operation-{}", journal.receipts.len() + 1),
            idempotency_key: command.key().to_owned(),
            request_sha256,
            subject: actor.subject.clone(),
            operation: command.kind().to_owned(),
            result_id: outcome,
            sequence: journal.receipts.len() as u64 + 1,
        };
        journal.receipts.push(receipt.clone());
        Ok(receipt)
    }

    pub fn inspect_policy(
        &self,
        access: PolicyAccess<'_>,
        reference: &SavedPolicyRef,
    ) -> Result<SavedPolicy, PolicyOwnerError> {
        self.read(access, PolicyPermission::ReadContent, |journal| {
            journal.policy(reference).cloned()
        })
    }

    pub fn inspect_review(
        &self,
        access: PolicyAccess<'_>,
        review_id: &str,
    ) -> Result<PolicyReview, PolicyOwnerError> {
        self.read(access, PolicyPermission::ReadMetadata, |journal| {
            journal.review(review_id).cloned()
        })
    }

    pub fn active_binding(
        &self,
        access: PolicyAccess<'_>,
    ) -> Result<Option<ActivePolicyBinding>, PolicyOwnerError> {
        self.read(access, PolicyPermission::ReadMetadata, |journal| {
            Ok(journal.active.clone())
        })
    }

    pub fn lookup_receipt(
        &self,
        access: PolicyAccess<'_>,
        key: &str,
    ) -> Result<PolicyReceipt, PolicyOwnerError> {
        self.authority.with_authorized(
            access,
            PolicyPermission::ReadMetadata,
            |state, actor, clock| {
                let store = self
                    .store
                    .lock()
                    .map_err(|_| PolicyOwnerError::Unavailable)?;
                let journal = store.load()?;
                let receipt = journal
                    .receipts
                    .iter()
                    .find(|receipt| {
                        receipt.subject == actor.subject && receipt.idempotency_key == key
                    })
                    .cloned()
                    .ok_or(PolicyOwnerError::Missing)?;
                state.check_actor(actor, PolicyPermission::ReadMetadata, clock.now_seconds())?;
                Ok(receipt)
            },
        )
    }

    fn read<T>(
        &self,
        access: PolicyAccess<'_>,
        permission: PolicyPermission,
        action: impl FnOnce(&PolicyJournal) -> Result<T, PolicyOwnerError>,
    ) -> Result<T, PolicyOwnerError> {
        self.authority
            .with_authorized(access, permission, |state, actor, clock| {
                let store = self
                    .store
                    .lock()
                    .map_err(|_| PolicyOwnerError::Unavailable)?;
                let result = action(&store.load()?)?;
                state.check_actor(actor, permission, clock.now_seconds())?;
                Ok(result)
            })
    }
}
