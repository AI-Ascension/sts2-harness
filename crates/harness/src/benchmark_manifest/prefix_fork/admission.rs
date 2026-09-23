// SPDX-License-Identifier: MIT

//! Effect-free admission of a fork from a verified prefix boundary.

use super::binding::ForkBinding;
use super::boundary::{LegalBinding, PrefixBoundary, ReplayObservation};
use super::error::PrefixForkRefusal;
use super::label_ok;

/// One request to fork from a verified seeded replay prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixForkRequest {
    /// Experiment the child branch belongs to.
    pub experiment_id: String,
    /// Stable identity reserved for the child continuation.
    pub continuation_id: String,
    /// Binding the operator requests for the new child branch.
    pub requested: ForkBinding,
    /// Binding recorded with the source prefix being replayed.
    pub recorded: ForkBinding,
    /// The selected settled boundary.
    pub boundary: PrefixBoundary,
    /// Observed result of replaying the prefix up to the boundary.
    pub replay: ReplayObservation,
}

/// An admitted fork; the prefix replay and child handoff may proceed against this boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixForkPlan {
    /// Experiment the child branch belongs to.
    pub experiment_id: String,
    /// Stable identity reserved for the child continuation.
    pub continuation_id: String,
    /// Immutable binding the child inherits from the source prefix.
    pub binding: ForkBinding,
    /// Selected boundary ordinal within the recorded prefix.
    pub boundary_ordinal: u32,
    /// Digest of the exact state captured at the boundary.
    pub state_digest: String,
    /// Stable action key bound at the boundary.
    pub action_key: String,
}

/// Admits a fork from a verified prefix boundary, refusing every incoherent request.
///
/// The checks run in a fixed order so the first failing property is reported: labels, the boundary
/// declaration, the requested-versus-recorded binding, terminality, settledness, receipt
/// completeness, the legal binding, and finally the observed replay. Every refusal happens before
/// the replay mutation or provider invocation it guards.
///
/// # Errors
///
/// Returns the specific [`PrefixForkRefusal`] for the first failing property; see the order above.
pub fn admit_prefix_fork(request: PrefixForkRequest) -> Result<PrefixForkPlan, PrefixForkRefusal> {
    if !label_ok(&request.experiment_id) || !label_ok(&request.continuation_id) {
        return Err(PrefixForkRefusal::InvalidLabel);
    }
    if request.requested.validate().is_err() || request.recorded.validate().is_err() {
        return Err(PrefixForkRefusal::InvalidLabel);
    }
    request.boundary.validate()?;
    let reasons = request.requested.compare(&request.recorded);
    if !reasons.is_empty() {
        return Err(PrefixForkRefusal::BindingMismatch { reasons });
    }
    if request.boundary.terminal {
        return Err(PrefixForkRefusal::TerminalSource);
    }
    if !request.boundary.settled {
        return Err(PrefixForkRefusal::UnresolvedAction);
    }
    let present = request.boundary.receipts.len();
    if present != request.boundary.expected_receipts as usize {
        return Err(PrefixForkRefusal::MissingReceipt {
            expected: request.boundary.expected_receipts,
            present,
        });
    }
    let action_key = match &request.boundary.legal_binding {
        LegalBinding::Resolved { action_key } => action_key.clone(),
        LegalBinding::Ambiguous => return Err(PrefixForkRefusal::AmbiguousLegalBinding),
    };
    check_replay(request.boundary.ordinal, request.replay)?;
    Ok(PrefixForkPlan {
        experiment_id: request.experiment_id,
        continuation_id: request.continuation_id,
        binding: request.recorded,
        boundary_ordinal: request.boundary.ordinal,
        state_digest: request.boundary.state_digest,
        action_key,
    })
}

fn check_replay(ordinal: u32, replay: ReplayObservation) -> Result<(), PrefixForkRefusal> {
    match replay {
        ReplayObservation::Verified {
            settled_actions,
            provider_calls,
        } => {
            if provider_calls != 0 {
                return Err(PrefixForkRefusal::ProviderCallsDuringReplay {
                    calls: provider_calls,
                });
            }
            if settled_actions != ordinal {
                return Err(PrefixForkRefusal::PrefixIncomplete {
                    replayed: settled_actions,
                    expected: ordinal,
                });
            }
            Ok(())
        }
        ReplayObservation::Diverged { ordinal: at } => {
            Err(PrefixForkRefusal::ObservedDivergence { ordinal: at })
        }
        ReplayObservation::Incomplete { replayed, expected } => {
            Err(PrefixForkRefusal::PrefixIncomplete { replayed, expected })
        }
    }
}
