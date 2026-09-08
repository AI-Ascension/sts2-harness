// SPDX-License-Identifier: MIT

//! The one durable-store owner shared by the worker control and execution paths.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use crate::{ExecutionFingerprint, ExecutionLineage, ExecutionStore, ResumeState, StoredEpisode};

/// The store and its process-local admission fence are shared as one opaque value. Callers must
/// use one of the lease helpers below; keeping the inner `Arc<Mutex<...>>` private prevents a
/// control or runtime path from accidentally bypassing the fence.
#[derive(Clone)]
pub struct SharedExecutionStore {
    store: Arc<Mutex<ExecutionStore>>,
    gate: Arc<AtomicU8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuarantineReason {
    InterruptedUnknown,
}

impl QuarantineReason {
    const fn label(self) -> &'static str {
        match self {
            Self::InterruptedUnknown => "interrupted-unknown quarantine",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GateState {
    Open,
    QuarantinePending(QuarantineReason),
    Quarantined(QuarantineReason),
    Corrupt,
}

impl GateState {
    const OPEN: u8 = 0;
    const QUARANTINE_PENDING: u8 = 1;
    const QUARANTINED: u8 = 2;
    const CORRUPT: u8 = 3;

    const fn code(self) -> u8 {
        match self {
            Self::Open => Self::OPEN,
            Self::QuarantinePending(_) => Self::QUARANTINE_PENDING,
            Self::Quarantined(_) => Self::QUARANTINED,
            Self::Corrupt => Self::CORRUPT,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            Self::OPEN => Self::Open,
            Self::QUARANTINE_PENDING => {
                Self::QuarantinePending(QuarantineReason::InterruptedUnknown)
            }
            Self::QUARANTINED => Self::Quarantined(QuarantineReason::InterruptedUnknown),
            _ => Self::Corrupt,
        }
    }

    fn blocked_message(self) -> String {
        match self {
            Self::Open => String::new(),
            Self::QuarantinePending(reason) => format!(
                "runtime-v3 execution store is fail-closed while {} persistence is pending",
                reason.label()
            ),
            Self::Quarantined(reason) => format!(
                "runtime-v3 execution store is fail-closed after {}",
                reason.label()
            ),
            Self::Corrupt => String::from(
                "runtime-v3 execution store is fail-closed because its gate is invalid",
            ),
        }
    }
}

impl SharedExecutionStore {
    /// Status only; callers must still acquire a checked lease for admission.
    pub(crate) fn admission_open(&self) -> bool {
        self.gate_state() == GateState::Open
    }

    fn gate_state(&self) -> GateState {
        GateState::from_code(self.gate.load(Ordering::Acquire))
    }

    /// Latches the process-local gate before the caller attempts the durable quarantine write.
    /// This intentionally never stores the free-form caller reason in shared memory.
    fn begin_quarantine(&self) -> Result<bool, String> {
        let pending = GateState::QuarantinePending(QuarantineReason::InterruptedUnknown).code();
        loop {
            match self.gate_state() {
                GateState::Open => {
                    if self
                        .gate
                        .compare_exchange(
                            GateState::Open.code(),
                            pending,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return Ok(false);
                    }
                }
                GateState::QuarantinePending(_) => return Ok(false),
                GateState::Quarantined(_) => return Ok(true),
                GateState::Corrupt => {
                    return Err(String::from(
                        "runtime-v3 execution store is fail-closed because its gate is invalid",
                    ));
                }
            }
        }
    }

    fn finish_quarantine(&self) {
        self.gate.store(
            GateState::Quarantined(QuarantineReason::InterruptedUnknown).code(),
            Ordering::Release,
        );
    }
}

pub fn share_store(store: ExecutionStore) -> SharedExecutionStore {
    SharedExecutionStore {
        store: Arc::new(Mutex::new(store)),
        gate: Arc::new(AtomicU8::new(GateState::Open.code())),
    }
}

/// A short store lease. The gate is checked before and after acquiring the underlying mutex for
/// admission-capable operations; the second check closes the race with a quarantine latch.
pub struct StoreLease<'a> {
    guard: MutexGuard<'a, ExecutionStore>,
}

impl Deref for StoreLease<'_> {
    type Target = ExecutionStore;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for StoreLease<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

/// The durable identity and resume boundary observed during one short store lease.
///
/// A runtime handle must not assemble these values from separate leases: another shared path may
/// advance the current attempt or checkpoint between those reads. Keeping the episode, resume
/// state, and checkpoint in one snapshot lets callers reject a stale or cross-lineage handle
/// before it can perform an effect.
pub struct StoreSnapshot {
    pub episode: StoredEpisode,
    pub resume: ResumeState,
}

pub fn snapshot(
    store: &ExecutionStore,
    lineage: &ExecutionLineage,
    fingerprint: &ExecutionFingerprint,
) -> Result<StoreSnapshot, String> {
    let resume = store
        .resume_episode(&lineage.episode_id, fingerprint)
        .map_err(|error| format!("cannot inspect runtime-v3 execution state: {error}"))?;
    if matches!(resume, ResumeState::New) {
        return Err(String::from(
            "runtime-v3 execution episode is missing from the admitted store",
        ));
    }
    let episode = store
        .load_episode(&lineage.episode_id)
        .map_err(|error| format!("cannot load runtime-v3 execution episode: {error}"))?;
    if episode.lineage != *lineage {
        return Err(String::from(
            "runtime-v3 stored episode lineage does not match the approved runtime",
        ));
    }
    if episode.fingerprint != *fingerprint {
        return Err(String::from(
            "runtime-v3 stored episode fingerprint does not match the approved runtime",
        ));
    }
    if let ResumeState::Ready { checkpoint, .. } = &resume
        && checkpoint.as_deref() != episode.last_checkpoint.as_ref()
    {
        return Err(String::from(
            "runtime-v3 stored resume boundary does not match the latest checkpoint",
        ));
    }
    Ok(StoreSnapshot { episode, resume })
}

/// Takes a bounded, non-blocking store lease. The lease must never span provider, MCP, or other
/// external I/O; callers hold it only for one short durable operation.
pub fn try_lock(store: &SharedExecutionStore) -> Result<StoreLease<'_>, String> {
    let state = store.gate_state();
    if state != GateState::Open {
        return Err(state.blocked_message());
    }
    let guard = match store.store.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(String::from(
            "runtime-v3 execution store is busy; retry the durable operation",
        )),
        Err(TryLockError::Poisoned(_)) => {
            Err(String::from("runtime-v3 execution store is poisoned"))
        }
    }?;
    let state = store.gate_state();
    if state != GateState::Open {
        drop(guard);
        return Err(state.blocked_message());
    }
    Ok(StoreLease { guard })
}

/// Read-only diagnostics and authoritative recovery/evidence accounting remain available after
/// the admission fence trips. Callers must not use this lease to allocate a new mutation.
pub fn try_lock_recovery(store: &SharedExecutionStore) -> Result<StoreLease<'_>, String> {
    match store.store.try_lock() {
        Ok(guard) => Ok(StoreLease { guard }),
        Err(TryLockError::WouldBlock) => Err(String::from(
            "runtime-v3 execution store is busy; retry the durable operation",
        )),
        Err(TryLockError::Poisoned(_)) => {
            Err(String::from("runtime-v3 execution store is poisoned"))
        }
    }
}

/// Quarantine and close bypass the normal admission fence. Recovery/evidence accounting also uses
/// the explicitly named recovery lease above so it can retain authoritative receipts/unknowns
/// without reopening mutation admission.
pub fn try_lock_quarantine(store: &SharedExecutionStore) -> Result<StoreLease<'_>, String> {
    try_lock_recovery(store)
}

pub fn try_lock_close(store: &SharedExecutionStore) -> Result<StoreLease<'_>, String> {
    try_lock_recovery(store)
}

pub fn begin_quarantine(store: &SharedExecutionStore) -> Result<bool, String> {
    store.begin_quarantine()
}

pub fn finish_quarantine(store: &SharedExecutionStore) {
    store.finish_quarantine();
}
