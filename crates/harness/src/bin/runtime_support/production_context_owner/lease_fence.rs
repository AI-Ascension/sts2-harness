// SPDX-License-Identifier: MIT

use super::*;

/// The run's trusted runtime lease, kept as a durable sidecar next to the
/// scoped context store. It survives an owner restart, so a superseded or
/// foreign lease can never win the first observation of a recovered run.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct TrustedLease {
    lease_id: String,
    lease_epoch: u64,
}

/// Enforce the trusted-lease fence on the first-contact path. A strictly newer
/// epoch is a legitimate post-restart re-allocation and advances the fence; an
/// equal epoch requires the same lease identity; anything older is superseded
/// and refused. The very first observation of a run establishes the fence,
/// since no caller-independent anchor exists before then.
pub(super) fn admit_trusted_lease(
    path: &std::path::Path,
    binding: &RuntimeAuthorityBinding,
) -> Result<(), ManagementError> {
    let trusted = match read_trusted_lease(path)? {
        Some(trusted) => trusted,
        None => {
            return store_trusted_lease(path, &trusted_lease_from(binding));
        }
    };
    let same_lease = trusted.lease_id == binding.lease_id;
    if binding.lease_epoch == trusted.lease_epoch {
        return if same_lease {
            Ok(())
        } else {
            Err(superseded_lease_error())
        };
    }
    if binding.lease_epoch < trusted.lease_epoch {
        return Err(superseded_lease_error());
    }
    store_trusted_lease(path, &trusted_lease_from(binding))
}

fn superseded_lease_error() -> ManagementError {
    ManagementError::conflict(
        "context_owner_runtime_scope",
        "runtime lease was superseded or does not belong to this workflow run",
    )
}

fn trusted_lease_from(binding: &RuntimeAuthorityBinding) -> TrustedLease {
    TrustedLease {
        lease_id: binding.lease_id.clone(),
        lease_epoch: binding.lease_epoch,
    }
}

pub(super) fn trusted_lease_path(base: &std::path::Path, run_id: &str) -> std::path::PathBuf {
    let mut path = scoped_store_path(base, run_id);
    path.set_extension("lease.json");
    path
}

fn read_trusted_lease(path: &std::path::Path) -> Result<Option<TrustedLease>, ManagementError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            ManagementError::unavailable("context_owner_lease_record", error.to_string())
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ManagementError::unavailable(
            "context_owner_lease_record",
            error.to_string(),
        )),
    }
}

fn store_trusted_lease(
    path: &std::path::Path,
    lease: &TrustedLease,
) -> Result<(), ManagementError> {
    let bytes = serde_json::to_vec(lease).map_err(|error| {
        ManagementError::unavailable("context_owner_lease_record", error.to_string())
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ManagementError::unavailable("context_owner_lease_record", error.to_string())
        })?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, &bytes).map_err(|error| {
        ManagementError::unavailable("context_owner_lease_record", error.to_string())
    })?;
    std::fs::rename(&temporary, path).map_err(|error| {
        ManagementError::unavailable("context_owner_lease_record", error.to_string())
    })
}
