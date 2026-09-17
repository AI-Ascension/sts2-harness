// SPDX-License-Identifier: MIT

/// Loads and validates one explicitly selected ready branch without performing external effects.
///
/// The returned metadata revision is the CAS token a runtime consumer must use when it claims the
/// continuation. Strategy assurance in the stored row confirms prior preparation only; the runtime
/// must obtain and verify evidence for the destination it is about to use.
///
/// # Errors
///
/// Returns a typed error for an unknown branch, a non-ready status, mismatched assurance, malformed
/// strategy source fields, absent/ambiguous source artifacts, or unavailable retained artifacts.
pub fn admit_branch_continuation(
    store: &SqliteBranchStore,
    selector: &BranchContinuationSelector,
    resolver: &dyn BranchArtifactResolver,
) -> Result<BranchContinuationAdmission, BranchContinuationAdmissionError> {
    let branch = store
        .get(selector.experiment_id(), selector.branch_id())
        .map_err(BranchContinuationAdmissionError::Store)?
        .ok_or(BranchContinuationAdmissionError::Store(
            BranchStoreError::UnknownBranch,
        ))?;
    if branch.status != DurableBranchStatus::Ready {
        return Err(BranchContinuationAdmissionError::BranchNotReady {
            status: branch.status,
        });
    }

    let strategy = strategy_plan(&branch)?;
    let artifacts = store
        .artifact_availability(selector.experiment_id(), selector.branch_id(), resolver)
        .map_err(BranchContinuationAdmissionError::Store)?;
    if !artifacts.all_available() {
        return Err(BranchContinuationAdmissionError::ArtifactsUnavailable(
            artifacts,
        ));
    }
    Ok(BranchContinuationAdmission {
        branch,
        strategy,
        artifacts,
    })
}

/// Admits a selected running branch for explicit execution-store resume.
///
/// This path requires the strategy-specific verified boundary assurance and every retained
/// artifact to remain available. It does not replay a prefix or reapply an exact restore. A
/// runtime caller must additionally prove the same live gateway owner and historical continuation
/// claim before opening the decision path.
///
/// # Errors
///
/// Returns a typed error when the branch is not running, lacks its strategy assurance, or has
/// unavailable retained artifacts.
pub fn admit_running_branch_continuation(
    store: &SqliteBranchStore,
    selector: &BranchContinuationSelector,
    resolver: &dyn BranchArtifactResolver,
) -> Result<BranchContinuationAdmission, BranchContinuationAdmissionError> {
    let branch = store
        .get(selector.experiment_id(), selector.branch_id())
        .map_err(BranchContinuationAdmissionError::Store)?
        .ok_or(BranchContinuationAdmissionError::Store(
            BranchStoreError::UnknownBranch,
        ))?;
    if branch.status != DurableBranchStatus::Running {
        return Err(BranchContinuationAdmissionError::BranchNotReady {
            status: branch.status,
        });
    }
    let strategy = strategy_plan(&branch)?;
    if branch.strategy == BranchStrategy::ExactRestore {
        // A Running exact branch is playable only after the independently
        // verified destination receipt was retained. The source checkpoint
        // assurance alone describes the input artifact and cannot authorize
        // a resumed destination.
        unique_artifact(&branch.artifacts, BranchArtifactRole::ContextSnapshot)?;
    }
    let artifacts = store
        .artifact_availability(selector.experiment_id(), selector.branch_id(), resolver)
        .map_err(BranchContinuationAdmissionError::Store)?;
    if !artifacts.all_available() {
        return Err(BranchContinuationAdmissionError::ArtifactsUnavailable(
            artifacts,
        ));
    }
    Ok(BranchContinuationAdmission {
        branch,
        strategy,
        artifacts,
    })
}

fn strategy_plan(
    branch: &DurableBranch,
) -> Result<BranchContinuationStrategyPlan, BranchContinuationAdmissionError> {
    let expected_assurance = match branch.strategy {
        BranchStrategy::ExactRestore => BranchAssurance::ExactRestoreReceipt,
        BranchStrategy::PrefixReplay => BranchAssurance::PrefixReplayBoundary,
    };
    if branch.assurance != expected_assurance {
        return Err(BranchContinuationAdmissionError::AssuranceMismatch {
            strategy: branch.strategy,
            assurance: branch.assurance,
        });
    }

    let (required_role, plan) = match branch.strategy {
        BranchStrategy::ExactRestore => {
            if branch.source_handle.is_none() || branch.trajectory_prefix.is_some() {
                return Err(
                    BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                        strategy: branch.strategy,
                    },
                );
            }
            let checkpoint = unique_artifact(&branch.artifacts, BranchArtifactRole::Checkpoint)?;
            let restore_closure = branch
                .artifacts
                .iter()
                .filter(|artifact| artifact.role == BranchArtifactRole::RestoreClosure)
                .cloned()
                .collect();
            (
                BranchArtifactRole::Checkpoint,
                BranchContinuationStrategyPlan::ExactRestore {
                    checkpoint,
                    restore_closure,
                },
            )
        }
        BranchStrategy::PrefixReplay => {
            if branch.trajectory_prefix.is_none() || branch.source_handle.is_some() {
                return Err(
                    BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                        strategy: branch.strategy,
                    },
                );
            }
            let replay_prefix =
                unique_artifact(&branch.artifacts, BranchArtifactRole::ReplayPrefix)?;
            (
                BranchArtifactRole::ReplayPrefix,
                BranchContinuationStrategyPlan::PrefixReplay { replay_prefix },
            )
        }
    };

    if branch
        .artifacts
        .iter()
        .any(|artifact| artifact.role == opposite_strategy_role(required_role))
    {
        return Err(
            BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                strategy: branch.strategy,
            },
        );
    }
    Ok(plan)
}

fn unique_artifact(
    artifacts: &[BranchArtifactReference],
    role: BranchArtifactRole,
) -> Result<BranchArtifactReference, BranchContinuationAdmissionError> {
    let mut matching = artifacts.iter().filter(|artifact| artifact.role == role);
    let Some(first) = matching.next() else {
        return Err(BranchContinuationAdmissionError::MissingStrategyArtifact { role });
    };
    if matching.next().is_some() {
        return Err(BranchContinuationAdmissionError::AmbiguousStrategyArtifact { role });
    }
    Ok(first.clone())
}

const fn opposite_strategy_role(role: BranchArtifactRole) -> BranchArtifactRole {
    match role {
        BranchArtifactRole::Checkpoint => BranchArtifactRole::ReplayPrefix,
        BranchArtifactRole::ReplayPrefix => BranchArtifactRole::Checkpoint,
        BranchArtifactRole::RestoreClosure | BranchArtifactRole::ContextSnapshot => role,
    }
}

fn valid_selector_part(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TRANSITION_LABEL_BYTES && !value.contains('\0')
}
