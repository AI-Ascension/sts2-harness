// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use super::*;

pub(super) fn reconcile_retained(
    roots: &[PolicyRoot],
    policy_locks: &[PolicyLock],
    policy: &ExoPrivateStatePolicy,
    policy_digest: &str,
) -> Result<(), &'static str> {
    let names = [
        policy_attempt_names(&roots[0], true)?,
        policy_attempt_names(&roots[1], true)?,
        policy_attempt_names(&roots[2], true)?,
    ];
    let expected = names
        .iter()
        .flat_map(|names| names.iter().cloned())
        .collect::<BTreeSet<_>>();
    for name in expected {
        verify_policy_locks(roots, policy_locks)?;
        let attempt_id = name.to_str().ok_or("exo_private_retained_mismatch")?;
        if !valid_attempt_id(attempt_id) {
            return Err("exo_private_retained_mismatch");
        }
        let attempts = roots
            .iter()
            .zip(&names)
            .map(|(root, names)| {
                names
                    .contains(&name)
                    .then(|| open_existing_attempt(root, &name))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let markers = attempts
            .iter()
            .map(|attempt| {
                attempt
                    .as_ref()
                    .map(|attempt| {
                        read_marker(&attempt.file).and_then(|bytes| {
                            serde_json::from_slice::<OwnerMarker>(&bytes)
                                .map_err(|_| "exo_private_marker")
                        })
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let service_uid = rustix::process::geteuid().as_raw();
        let first_index = markers
            .iter()
            .position(Option::is_some)
            .ok_or("exo_private_marker")?;
        let first = markers[first_index].as_ref().ok_or("exo_private_marker")?;
        if first.schema != MARKER_SCHEMA
            || first.attempt_id != attempt_id
            || first.policy_digest != policy_digest
            || first.service_uid != service_uid
            || !valid_digest(&first.config_digest)
            || !valid_boot_id(&first.boot_id)
            || first.root_proofs.len() != ROOT_KINDS.len()
            || first.phase != MarkerPhase::Quiescent
        {
            return Err("exo_private_retained_identity");
        }
        for (index, (marker, attempt)) in markers.iter().zip(&attempts).enumerate() {
            let (Some(marker), Some(attempt)) = (marker.as_ref(), attempt.as_ref()) else {
                if marker.is_none() && attempt.is_none() {
                    continue;
                }
                return Err("exo_private_retained_identity");
            };
            let proof = first
                .root_proofs
                .get(index)
                .ok_or("exo_private_retained_identity")?;
            if proof.kind != ROOT_KINDS[index]
                || proof.base_identity != roots[index].identity
                || proof.attempt_identity != attempt.identity
            {
                return Err("exo_private_retained_identity");
            }
            if marker.schema != first.schema
                || marker.attempt_id != first.attempt_id
                || marker.config_digest != first.config_digest
                || marker.policy_digest != first.policy_digest
                || marker.service_uid != first.service_uid
                || marker.created_at_unix_seconds != first.created_at_unix_seconds
                || marker.boot_id != first.boot_id
                || marker.root_proofs != first.root_proofs
                || marker.phase != first.phase
                || marker.process != first.process
                || marker.root_kind != ROOT_KINDS[index]
                || marker.root_identity != attempt.identity
            {
                return Err("exo_private_retained_identity");
            }
        }
        if let Some(process) = first.process.as_ref()
            && (process.uid != service_uid
                || process.pid != process.process_group
                || process.parent_pid == 0
                || process.session == 0
                || process.start_time_ticks == 0
                || process.boot_id != first.boot_id)
        {
            return Err("exo_private_retained_identity");
        }
        let age = unix_seconds()?.checked_sub(first.created_at_unix_seconds);
        let retention = u64::from(policy.max_retention_days)
            .checked_mul(24 * 60 * 60)
            .ok_or("exo_private_retention")?;
        if age.is_some_and(|age| age >= retention) {
            for (root, attempt) in roots.iter().zip(&attempts) {
                if let Some(attempt) = attempt {
                    verify_policy_locks(roots, policy_locks)?;
                    remove_attempt(root, attempt, u64::MAX)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "retained_tests.rs"]
mod tests;
