// SPDX-License-Identifier: MIT

//! Fence binding for one served prepared-application approval.
//!
//! Every axis is computed from a value the served session or the owner-authored render source
//! actually publishes, so a held approval is bound to the identity it was prepared under rather
//! than to a placeholder. A value that is not itself a digest is bound through a canonical digest,
//! so the fence stays well formed whatever an owner publishes while any change in a contributing
//! value still makes a held approval stale.

use crate::context_capture::DispatchFences;
use crate::management::ContextRenderSourceIdentity;
use sha2::{Digest, Sha256};

/// The advertised exact adapter this served composition records.
///
/// The recording sink is attached to the Exo lane only, so the served release may claim exactness
/// for that adapter's advertised boundary and for no other adapter.
pub(super) const BOUNDARY_ADAPTER_ID: &str = "exo";

/// Domain separation for every fence binding this composition computes.
const FENCE_DOMAIN: &[u8] = b"ascension.served-boundary-fence.v1\0";

/// Canonical, length-prefixed binding over served fence values.
///
/// The output is 64 lowercase hexadecimal characters, so it is also a valid identity where the
/// fence vocabulary needs one.
fn fence_binding(label: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FENCE_DOMAIN);
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label.as_bytes());
    hasher.update((parts.len() as u64).to_be_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    crate::hex_bytes(hasher.finalize())
}

/// Binds every fence axis this served composition can observe.
///
/// `revocation_epoch` has no source in this composition: it is bound to the value the composition
/// has and is recorded as a residual instead of being claimed as coverage.
pub(super) fn boundary_fences(
    identity: &ContextRenderSourceIdentity,
    provider_revision: &str,
) -> DispatchFences {
    let boundary = &identity.boundary;
    let generation = boundary.generation.to_string();
    let control_version = boundary.control_version.to_string();
    let lease_epoch = identity.lease_epoch.to_string();
    let state = format!("{}:{generation}", boundary.state_id);
    DispatchFences {
        adapter_id: BOUNDARY_ADAPTER_ID.to_owned(),
        model_id: fence_binding("model", &[&boundary.model_revision, provider_revision]),
        configuration_digest: fence_binding(
            "configuration",
            &[
                &boundary.configuration_sha256,
                &boundary.adapter_revision,
                &boundary.output_schema_sha256,
            ],
        ),
        state_digest: fence_binding("state", &[&boundary.observation_sha256, &state]),
        catalog_digest: fence_binding("catalog", &[&boundary.catalog_sha256]),
        profile_digest: fence_binding(
            "profile",
            &[&identity.binding_digest, &identity.active_revision_id],
        ),
        auth_digest: fence_binding(
            "auth",
            &[&identity.instance_id, &identity.lease_id, &lease_epoch],
        ),
        history_digest: fence_binding("history", &[&identity.source_id, &identity.source_digest]),
        compaction_digest: fence_binding(
            "compaction",
            &[
                identity
                    .membership_digest
                    .as_deref()
                    .unwrap_or("<stateless>"),
                &control_version,
            ],
        ),
        policy_id: fence_binding("policy", &[&identity.owner_id, &identity.binding_id]),
        policy_version: identity.binding_version,
        controller_epoch: boundary.controller_epoch,
        gate_epoch: boundary.gate_epoch,
        lease_epoch: identity.lease_epoch,
        revocation_epoch: 0,
    }
}

/// Deterministic attempt identity for one served boundary approval.
///
/// The attempt is derived from the served invocation identity instead of a per-process counter so
/// that a repeated release of the same invocation resolves to the same captured snapshot.
pub(super) fn boundary_attempt_id(
    identity: &ContextRenderSourceIdentity,
    execution_id: &str,
) -> String {
    let binding = fence_binding("attempt", &[&identity.invocation_id, execution_id]);
    format!("exo-attempt.{}", &binding[..32])
}
