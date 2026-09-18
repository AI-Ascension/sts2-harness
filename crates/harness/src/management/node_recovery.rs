// SPDX-License-Identifier: MIT

use super::super::contract::PendingOperationState;
use super::super::service::ManagementError;
use super::execution_state::{LiveNodeState, PendingDispatch};
use crate::episode::{DispatchStatus, TransitionReceipt, WaitOutcome, WaitSample};
use crate::workflow::RuntimeFault;

pub(super) fn reconcile_pending(state: &mut LiveNodeState) -> Result<(), ManagementError> {
    let Some(pending) = state.pending.as_mut() else {
        return Ok(());
    };
    let receipt = state
        .session
        .reconcile(pending.identity.operation_id.as_str())?;
    if receipt.operation_id() != pending.identity.operation_id
        || receipt.action() != &pending.action
    {
        return Err(ManagementError::conflict(
            "live_reconcile_identity",
            "reconciliation returned a different operation or action",
        ));
    }
    match receipt.status() {
        DispatchStatus::Settled | DispatchStatus::Rejected | DispatchStatus::Cancelled => {
            pending.resolved = Some(receipt);
            Ok(())
        }
        DispatchStatus::Accepted | DispatchStatus::Unknown => {
            pending.state = PendingOperationState::Unknown;
            Err(ManagementError::unresolved(
                "live_operation_unknown",
                "accepted mutation remains unresolved; no replacement action is permitted",
            ))
        }
    }
}

pub(super) fn settled_receipt(
    pending: &PendingDispatch,
    sample: WaitSample,
) -> Result<TransitionReceipt, RuntimeFault> {
    if !matches!(
        sample.outcome(),
        WaitOutcome::Successor | WaitOutcome::SameStateMutation
    ) {
        return Err(RuntimeFault::UnknownEffect);
    }
    let after = sample
        .observation()
        .cloned()
        .ok_or(RuntimeFault::UnknownEffect)?;
    let effect = sample.effect_kind().ok_or(RuntimeFault::UnknownEffect)?;
    Ok(TransitionReceipt::new(
        pending.identity.operation_id.clone(),
        pending.action.clone(),
        DispatchStatus::Settled,
        Some(after),
        Some(effect.to_owned()),
        None,
    ))
}
