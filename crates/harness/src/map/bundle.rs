// SPDX-License-Identifier: MIT

use super::analysis::{AnalysisConfig, MAP_ANALYSIS_VERSION, MapAnalysis};
use super::bundle_validation_checks::validate_without_bundle_digest;
use super::canonical::{canonical_bytes, canonical_digest, reject_duplicate_keys};
use super::graph::ValidatedMapGraph;
use sha2::{Digest as _, Sha256};
use sts2_protocol::decode_map_snapshot;

pub use super::bundle_types::{
    BundleContents, BundleHistory, BundleManifest, BundleOrigin, BundlePresentation, MapViewBundle,
    RuntimeMapBundleIdentity,
};
pub use super::bundle_validation::MapBundleError;

pub const MAP_BUNDLE_VERSION: &str = "sts2.map-view-bundle-v1";
pub const RUNTIME_MAP_SCHEMA_DIGEST: &str =
    "6340f3cbe6c1b5728144fe89fdfdf8645acf2f59a77c0e0c30ebfeafc77515d8";
pub const MAP_MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
pub const MAP_MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAP_MIN_PRESENTATION_WIDTH: u32 = 320;
pub const MAP_MAX_PRESENTATION_WIDTH: u32 = 8192;
pub const MAP_MIN_PRESENTATION_HEIGHT: u32 = 320;
pub const MAP_MAX_PRESENTATION_HEIGHT: u32 = 16384;
pub const MAP_MAX_PRESENTATION_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAP_MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
pub const RUNTIME_MAP_UNRENDERED_DECISION: &[u8] =
    br#"{"kind":"map-analysis","dispatchable":false}"#;
pub(crate) const MAX_BUNDLE_TEXT_BYTES: usize = 512;

impl MapViewBundle {
    /// Builds the harness-owned unrendered runtime map artifact from one
    /// frozen visible snapshot. The returned viewer is the exact `{}` sentinel
    /// and carries no renderer or dispatch authority.
    pub fn from_runtime_snapshot(
        snapshot_bytes: Vec<u8>,
        identity: RuntimeMapBundleIdentity,
    ) -> Result<Self, MapBundleError> {
        if snapshot_bytes.len() > MAP_MAX_SNAPSHOT_BYTES {
            return Err(MapBundleError::TooLarge("snapshot"));
        }
        let snapshot_digest = crate::hex_bytes(Sha256::digest(&snapshot_bytes));
        decode_map_snapshot(&snapshot_bytes)
            .map_err(|error| MapBundleError::ProtocolSnapshot(error.to_string()))?;
        let graph = ValidatedMapGraph::from_visible_map_json(snapshot_digest, &snapshot_bytes)
            .map_err(MapBundleError::Graph)?;
        let analysis = MapAnalysis::analyze(&graph, AnalysisConfig::default())
            .map_err(MapBundleError::Analysis)?;
        let decision = RUNTIME_MAP_UNRENDERED_DECISION.to_vec();
        let viewer = b"{}".to_vec();
        let manifest = BundleManifest {
            bundle_version: MAP_BUNDLE_VERSION.to_owned(),
            bundle_digest: String::new(),
            snapshot_digest: graph.snapshot_digest().to_owned(),
            analysis_digest: analysis.content_digest.clone(),
            map_instance: graph.map_instance().to_owned(),
            act: graph.act().to_owned(),
            run_id: identity.run_id,
            episode_id: identity.episode_id,
            trajectory_id: identity.trajectory_id,
            model_execution_id: identity.model_execution_id,
            schema_profile: "runtime-map-v1".to_owned(),
            schema_digest: RUNTIME_MAP_SCHEMA_DIGEST.to_owned(),
            analysis_version: MAP_ANALYSIS_VERSION.to_owned(),
            renderer_version: "unrendered".to_owned(),
            presentation: None,
            origin: BundleOrigin {
                owner: "sts2-harness".to_owned(),
                source: "runtime-map-v1".to_owned(),
                generator: "sts2-harness-runtime-map-v1".to_owned(),
                license: "MIT".to_owned(),
            },
            contents: BundleContents {
                snapshot_ref: "visible-map.json".to_owned(),
                analysis_ref: "analysis.json".to_owned(),
                decision_ref: "decision.json".to_owned(),
                viewer_ref: "viewer.json".to_owned(),
                svg_ref: None,
                png_ref: None,
                svg_digest: None,
                png_digest: None,
                decision_digest: Some(crate::hex_bytes(Sha256::digest(&decision))),
                viewer_digest: Some(crate::hex_bytes(Sha256::digest(&viewer))),
            },
            history: BundleHistory {
                source_state_id: graph.source_state_id().to_owned(),
                generation: graph.generation(),
                action_catalog_digest: identity.action_catalog_digest,
            },
        };
        Self::new(
            manifest,
            snapshot_bytes,
            analysis,
            None,
            None,
            Some(decision),
            Some(viewer),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut manifest: BundleManifest,
        snapshot_bytes: Vec<u8>,
        analysis: MapAnalysis,
        svg: Option<Vec<u8>>,
        png: Option<Vec<u8>>,
        decision: Option<Vec<u8>>,
        viewer: Option<Vec<u8>>,
    ) -> Result<Self, MapBundleError> {
        manifest.bundle_digest.clear();
        let bundle = Self {
            manifest,
            snapshot_bytes,
            analysis,
            svg,
            png,
            decision,
            viewer,
        };
        validate_without_bundle_digest(&bundle)?;
        let mut result = bundle;
        result.manifest.bundle_digest = result.compute_digest()?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), MapBundleError> {
        validate_without_bundle_digest(self)?;
        if self.manifest.bundle_digest != self.compute_digest()? {
            return Err(MapBundleError::DigestMismatch("bundle_digest"));
        }
        Ok(())
    }

    fn compute_digest(&self) -> Result<String, MapBundleError> {
        let mut manifest = self.manifest.clone();
        manifest.bundle_digest.clear();
        canonical_digest(&manifest).map_err(MapBundleError::Canonical)
    }

    pub fn canonical_manifest_bytes(&self) -> Result<Vec<u8>, MapBundleError> {
        self.validate()?;
        canonical_bytes(&self.manifest).map_err(MapBundleError::Canonical)
    }

    pub fn decode_manifest(bytes: &[u8]) -> Result<BundleManifest, MapBundleError> {
        reject_duplicate_keys(bytes).map_err(MapBundleError::Canonical)?;
        let manifest: BundleManifest =
            serde_json::from_slice(bytes).map_err(|_| MapBundleError::Serialization)?;
        if manifest.bundle_version != MAP_BUNDLE_VERSION {
            return Err(MapBundleError::UnsupportedVersion(manifest.bundle_version));
        }
        Ok(manifest)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_manifest_and_files(
        manifest: BundleManifest,
        snapshot_bytes: Vec<u8>,
        analysis_bytes: &[u8],
        svg: Option<Vec<u8>>,
        png: Option<Vec<u8>>,
        decision: Option<Vec<u8>>,
        viewer: Option<Vec<u8>>,
    ) -> Result<Self, MapBundleError> {
        let analysis = MapAnalysis::decode(analysis_bytes).map_err(MapBundleError::Analysis)?;
        let bundle = Self {
            manifest,
            snapshot_bytes,
            analysis,
            svg,
            png,
            decision,
            viewer,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn analysis_bytes(&self) -> Result<Vec<u8>, MapBundleError> {
        self.analysis
            .canonical_bytes()
            .map_err(MapBundleError::Analysis)
    }

    #[must_use]
    pub fn bundle_digest(&self) -> &str {
        &self.manifest.bundle_digest
    }
}

/// Convenience wrapper for callers that do not need to name the bundle type.
pub fn build_unrendered_runtime_map_bundle(
    snapshot_bytes: Vec<u8>,
    identity: RuntimeMapBundleIdentity,
) -> Result<MapViewBundle, MapBundleError> {
    MapViewBundle::from_runtime_snapshot(snapshot_bytes, identity)
}
