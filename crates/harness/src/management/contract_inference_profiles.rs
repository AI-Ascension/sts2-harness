// SPDX-License-Identifier: MIT

//! The owner's published inference-profile admission decision.
//!
//! Definition validation and Studio publication are two admission decisions
//! about the same document, so both publish this one record instead of each
//! letting a consumer re-derive an admission rule of its own.

use serde::{Deserialize, Serialize};

/// One authored inference-profile reference and the exact revision the owner
/// resolved it to.
///
/// `profile_ref` is the reference exactly as authored — which may be a floating
/// id — and `resolved_pin` is the `profile_id:version:digest` identity it
/// resolved to. A consumer that wants immutability must record `resolved_pin`;
/// `profile_ref` alone is not an immutable identity, because a later catalog
/// revision could resolve the same floating id to a different descriptor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedInferenceProfileRef {
    pub graph_id: String,
    pub node_id: String,
    pub node_kind: String,
    pub profile_ref: String,
    pub resolved_pin: String,
    /// JSON path of the reference within the submitted definition, e.g.
    /// `$.graphs[0].nodes[1].config.planner_profile_ref`.
    pub path: String,
}
