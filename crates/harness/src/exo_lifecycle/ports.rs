// SPDX-License-Identifier: MIT

use super::{InvocationManifest, JournalConfig, LifecycleError, NativeIdentity};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimKind {
    Create,
    Restart,
    /// Caller authority must establish legacy quiescence and revoke the old execution route.
    ImportLegacy {
        source_digest: String,
        cutover_ref: String,
    },
}

pub struct OwnerClaim<'a> {
    pub config: &'a JournalConfig,
    pub kind: &'a ClaimKind,
}

/// A short, linearizable owner authorization guard. No default implementation is provided.
pub trait AuthorityGuard {}

/// Supplied by the authenticated scheduler. This source slice provides no production adapter.
pub trait LifecycleAuthorityPort: Send + Sync {
    fn claim<'a>(
        &'a self,
        request: &OwnerClaim<'_>,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError>;
    fn admit<'a>(
        &'a self,
        manifest: &InvocationManifest,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError>;
    fn consume<'a>(
        &'a self,
        manifest: &InvocationManifest,
        result_digest: &str,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError>;
}

/// Non-cloneable, non-serializable evidence of a durable send boundary.
pub struct SendPermit {
    pub(crate) operation_id: String,
    pub(crate) claim_epoch: u64,
    pub(crate) revision: u64,
}
impl SendPermit {
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn claim_epoch(&self) -> u64 {
        self.claim_epoch
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

pub struct EffectCompletion {
    /// Existing correlated bridge response envelope; never persisted in the metadata journal.
    pub response: Vec<u8>,
    pub result_ref: String,
    /// Qualified owner units only. Missing/zero/over-reservation usage cannot complete.
    pub actual_units: Option<u64>,
    pub native: Option<NativeIdentity>,
}

/// Poll is nonblocking. Waiting/model work belongs outside the short authority guard.
pub trait EffectHandle {
    fn poll(&mut self) -> Result<Option<EffectCompletion>, LifecycleError>;
}

pub trait EffectPort {
    type Handle: EffectHandle;
    /// Must hand off promptly without waiting for inference. Every error is ambiguous.
    fn try_start(
        &mut self,
        permit: SendPermit,
        input: &[u8],
    ) -> Result<Self::Handle, LifecycleError>;
}
