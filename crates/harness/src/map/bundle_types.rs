// SPDX-License-Identifier: MIT

use super::analysis::MapAnalysis;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeMapBundleIdentity {
    pub run_id: String,
    pub episode_id: String,
    pub trajectory_id: String,
    pub model_execution_id: Option<String>,
    pub action_catalog_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleOrigin {
    pub owner: String,
    pub source: String,
    pub generator: String,
    pub license: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleContents {
    pub snapshot_ref: String,
    pub analysis_ref: String,
    pub decision_ref: String,
    pub viewer_ref: String,
    pub svg_ref: Option<String>,
    pub png_ref: Option<String>,
    pub svg_digest: Option<String>,
    pub png_digest: Option<String>,
    pub decision_digest: Option<String>,
    pub viewer_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleHistory {
    pub source_state_id: String,
    pub generation: u64,
    pub action_catalog_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundlePresentation {
    pub width: u32,
    pub height: u32,
    pub layout_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub bundle_version: String,
    /// Optional public reference; present only in the explicitly versioned v2 bundle.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::checkpoint_projection::deserialize_optional_reference"
    )]
    pub checkpoint_reference: Option<crate::PublicCheckpointSummary>,
    pub bundle_digest: String,
    pub snapshot_digest: String,
    pub analysis_digest: String,
    pub map_instance: String,
    pub act: String,
    pub run_id: String,
    pub episode_id: String,
    pub trajectory_id: String,
    pub model_execution_id: Option<String>,
    pub schema_profile: String,
    pub schema_digest: String,
    pub analysis_version: String,
    pub renderer_version: String,
    pub presentation: Option<BundlePresentation>,
    pub origin: BundleOrigin,
    pub contents: BundleContents,
    pub history: BundleHistory,
}

/// In-memory bundle material. Publication writes a manifest and separate files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MapViewBundle {
    pub manifest: BundleManifest,
    pub snapshot_bytes: Vec<u8>,
    pub analysis: MapAnalysis,
    pub svg: Option<Vec<u8>>,
    pub png: Option<Vec<u8>>,
    pub decision: Option<Vec<u8>>,
    pub viewer: Option<Vec<u8>>,
}
