// SPDX-License-Identifier: MIT

use super::JournalSnapshot;
use crate::exo_lifecycle::{LifecycleEntry, LifecycleError, LifecyclePhase};
use crate::provider_session::{
    BindingState, NativeOperation, NativeOperationKind, NativeOperationState, SessionBinding,
};

pub(super) fn validate(
    snapshot: &JournalSnapshot,
    entry: &LifecycleEntry,
) -> Result<(), LifecycleError> {
    let manifest = &entry.manifest;
    let mut bindings = snapshot
        .broker
        .bindings
        .iter()
        .filter(|binding| binding.binding_id == manifest.binding_id);
    let binding = bindings.next().ok_or(LifecycleError::Corrupt)?;
    let mut operations = snapshot
        .broker
        .operations
        .iter()
        .filter(|operation| operation.operation_id == manifest.operation_id);
    let operation = operations.next().ok_or(LifecycleError::Corrupt)?;
    let authority = &manifest.authority;
    if bindings.next().is_some()
        || operations.next().is_some()
        || binding.scope != manifest.scope
        || binding.owner_epoch != snapshot.claim_epoch
        || binding.session_epoch < authority.session_epoch
        || binding.history_epoch < authority.history_epoch
        || binding.compaction_epoch < authority.compaction_epoch
        || binding.profile_sha256 != manifest.profile_digest
        || operation.scope != manifest.scope
        || operation.binding_id != manifest.binding_id
        || operation.kind != NativeOperationKind::Turn
        || operation.request_sha256 != manifest.operation_digest()?
        || operation.owner_epoch != entry.claim_epoch
        || authority.auth_epoch != entry.claim_epoch
        || operation.session_epoch != authority.session_epoch
        || snapshot.broker.revocation_epoch < authority.revocation_epoch
        || !operation.generation_permission
        || !operation.generation_class
        || operation.automatic_retry
        || operation.auto_resume
        || operation.game_effects != 0
    {
        return Err(LifecycleError::Corrupt);
    }
    phase(snapshot, entry, binding, operation)
}

fn phase(
    snapshot: &JournalSnapshot,
    entry: &LifecycleEntry,
    binding: &SessionBinding,
    operation: &NativeOperation,
) -> Result<(), LifecycleError> {
    use LifecyclePhase as Phase;
    use NativeOperationState as State;
    let historical = entry.claim_epoch < snapshot.claim_epoch;
    let active = binding.state == BindingState::Active && binding.game_dispatch_capability;
    let recovering = binding.state == BindingState::Recovering && !binding.game_dispatch_capability;
    let compatible = match entry.phase {
        Phase::Prepared | Phase::Admitted => {
            if historical {
                operation.state == State::Unknown && recovering
            } else {
                operation.state == State::IntentPersisted && active
            }
        }
        Phase::Sent => !historical && operation.state == State::Sent && active,
        Phase::Unknown | Phase::Fenced => operation.state == State::Unknown && recovering,
        Phase::Completed => {
            if let Some(native) = &entry.native {
                // Later turns/recovery may change the binding; the completed operation is
                // immutable evidence. Do not require its binding to remain Active forever.
                operation.state == State::Completed
                    && operation.terminal_evidence_ref.as_deref() == Some(&native.turn_id)
            } else {
                // Explicit completed-store repair does not complete the broker operation.
                (operation.state == State::Unknown && recovering)
                    || (!historical && operation.state == State::Sent && active)
            }
        }
        // No writer in this source slice produces this reserved future phase.
        Phase::FailedBeforeSend => false,
    };
    if !compatible
        || (entry.phase == Phase::Fenced && !entry.possible_write)
        || (entry.native.is_none() && operation.terminal_evidence_ref.is_some())
        || (entry.phase != Phase::Completed
            && (entry.native.is_some() || entry.result_ref.is_some()))
        || (entry.phase != Phase::Completed
            && (binding.session_epoch != entry.manifest.authority.session_epoch
                || binding.history_epoch != entry.manifest.authority.history_epoch
                || binding.compaction_epoch != entry.manifest.authority.compaction_epoch))
    {
        return Err(LifecycleError::Corrupt);
    }
    Ok(())
}
