// SPDX-License-Identifier: MIT

//! Harness consumer for the pinned exact-restore profile.

use serde_json::Value;

#[path = "exact_restore_closure.rs"]
mod closure;
#[path = "exact_restore_pins.rs"]
mod pins;
#[path = "exact_restore_protocol.rs"]
mod protocol;

#[path = "exact_restore_operation.rs"]
pub(crate) mod operation;

pub(super) use closure::{TransferBlob, VerifiedClosure};
pub(crate) use operation::execute;
pub(crate) use pins::ProfilePins;
#[cfg(test)]
pub(crate) use pins::validate_pin;
#[cfg(test)]
pub(super) use protocol::wrapper_validator;
pub(super) use protocol::{
    MAX_CHUNK_BYTES, canonical_bytes, encode_base64, neutral_validator, valid_schema,
    wrapper_request,
};

pub(crate) const NEUTRAL_CONTRACT: &str = "sts2-exact-restore-v1";
pub(crate) const NEUTRAL_SCHEMA_DIGEST: &str =
    "2289d888c33eac46873408303c4423eab762e3f7bd6132ae8ae88d0d3b1858e4";
pub(crate) const WRAPPER_CONTRACT: &str = "sts2-exact-restore-gateway-v1";
pub(crate) const WRAPPER_SCHEMA_DIGEST: &str =
    "0b181dc30524c8b14dea73e490da55538f2d57fe87bf58ed9fe33223406a7d89";
pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024;
pub(crate) const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_ARTIFACT_REFERENCES: usize = 64;
pub(crate) const MAX_CLOSURE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const CLOSURE_DOMAIN: &[u8] = b"STS2/EXACT-RESTORE-CLOSURE/v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureSafety {
    /// A typed rejection guarantees that no host mutation began.
    NotStarted,
    /// The operation could have staged bytes or started a host effect.
    Uncertain,
}

#[derive(Debug)]
pub(crate) struct ExactRestoreError {
    pub(crate) safety: FailureSafety,
    pub(crate) message: String,
}

pub(super) fn branch_payload(
    selected: &super::branch_continuation_runtime::SelectedBranchContinuation,
) -> Value {
    let branch = selected.branch();
    serde_json::json!({
        "experiment_id": branch.experiment_id,
        "branch_id": branch.branch_id,
        "metadata_revision": branch.metadata_revision,
        "run_id": branch.run_id,
        "episode_id": branch.episode_id,
        "trajectory_id": branch.trajectory_id,
    })
}

#[cfg(test)]
#[path = "exact_restore_tests.rs"]
mod tests;
