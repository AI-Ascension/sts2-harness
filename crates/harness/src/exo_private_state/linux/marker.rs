// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::super::fs::FileIdentity;

pub(super) const MARKER_SCHEMA: &str = "sts2.exo-private-state-owner-v2";
pub(super) const ROOT_KINDS: [&str; 3] = ["state", "cache", "temp"];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MarkerPhase {
    Preparing,
    Starting,
    Running,
    Quiescent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RootProof {
    pub kind: String,
    pub base_identity: FileIdentity,
    pub attempt_identity: FileIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerMarker {
    pub schema: String,
    pub attempt_id: String,
    pub config_digest: String,
    pub policy_digest: String,
    pub service_uid: u32,
    pub created_at_unix_seconds: u64,
    pub boot_id: String,
    pub root_kind: String,
    pub root_proofs: Vec<RootProof>,
    pub root_identity: FileIdentity,
    pub phase: MarkerPhase,
    pub process: Option<super::super::ProcessIdentity>,
}
