// SPDX-License-Identifier: MIT

use super::analysis::{AnalysisConfig, MAP_ANALYSIS_VERSION, MapAnalysis};
use super::bundle_validation::{
    check_digest, is_digest, validate_file_reference, validate_json_object,
    validate_snapshot_document, verify_optional_digest, verify_required_digest,
};
use super::canonical::{canonical_bytes, canonical_digest, reject_duplicate_keys};
use super::graph::ValidatedMapGraph;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

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
const MAX_BUNDLE_TEXT_BYTES: usize = 512;

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
        bundle.validate_without_bundle_digest()?;
        let mut result = bundle;
        result.manifest.bundle_digest = result.compute_digest()?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), MapBundleError> {
        self.validate_without_bundle_digest()?;
        if self.manifest.bundle_digest != self.compute_digest()? {
            return Err(MapBundleError::DigestMismatch("bundle_digest"));
        }
        Ok(())
    }

    fn validate_without_bundle_digest(&self) -> Result<(), MapBundleError> {
        if self.snapshot_bytes.len() > MAP_MAX_SNAPSHOT_BYTES {
            return Err(MapBundleError::TooLarge("snapshot"));
        }
        if self
            .png
            .as_ref()
            .is_some_and(|bytes| bytes.len() > MAP_MAX_PNG_BYTES)
        {
            return Err(MapBundleError::TooLarge("png"));
        }
        if self.manifest.bundle_version != MAP_BUNDLE_VERSION {
            return Err(MapBundleError::UnsupportedVersion(
                self.manifest.bundle_version.clone(),
            ));
        }
        self.validate_manifest()?;
        self.analysis
            .verify_digest()
            .map_err(MapBundleError::Analysis)?;
        if self.analysis.snapshot_digest != self.manifest.snapshot_digest {
            return Err(MapBundleError::DigestMismatch("analysis.snapshot_digest"));
        }
        check_digest(
            "snapshot_digest",
            &self.manifest.snapshot_digest,
            &self.snapshot_bytes,
        )?;
        validate_snapshot_document(
            &self.snapshot_bytes,
            &self.manifest.schema_profile,
            &self.manifest.history,
            &self.manifest.map_instance,
            &self.manifest.act,
        )?;
        if self.manifest.analysis_digest != self.analysis.content_digest {
            return Err(MapBundleError::DigestMismatch("analysis_digest"));
        }
        match (
            self.manifest.renderer_version.as_str(),
            self.viewer.as_deref(),
        ) {
            ("unrendered", Some(bytes)) if bytes == b"{}" => {}
            ("unrendered", _) => {
                return Err(MapBundleError::InvalidField("unrendered viewer"));
            }
            (_, Some(bytes)) if bytes == b"{}" => {
                return Err(MapBundleError::InvalidField("rendered viewer"));
            }
            (_, Some(_)) => {}
            (_, None) => return Err(MapBundleError::DigestMismatch("viewer_digest")),
        }
        match (
            self.manifest.renderer_version.as_str(),
            self.manifest.presentation.as_ref(),
        ) {
            ("unrendered", None) => {}
            ("unrendered", Some(_)) => {
                return Err(MapBundleError::InvalidField("unrendered presentation"));
            }
            (_, Some(settings))
                if (MAP_MIN_PRESENTATION_WIDTH..=MAP_MAX_PRESENTATION_WIDTH)
                    .contains(&settings.width)
                    && (MAP_MIN_PRESENTATION_HEIGHT..=MAP_MAX_PRESENTATION_HEIGHT)
                        .contains(&settings.height)
                    && u64::from(settings.width)
                        .checked_mul(u64::from(settings.height))
                        .is_some_and(|pixels| pixels <= MAP_MAX_PRESENTATION_PIXELS)
                    && !settings.layout_version.is_empty()
                    && settings.layout_version.len() <= MAX_BUNDLE_TEXT_BYTES => {}
            (_, _) => return Err(MapBundleError::InvalidField("presentation")),
        }
        if self.manifest.contents.snapshot_ref != "visible-map.json"
            || self.manifest.contents.analysis_ref != "analysis.json"
            || self.manifest.contents.decision_ref != "decision.json"
            || self.manifest.contents.viewer_ref != "viewer.json"
        {
            return Err(MapBundleError::InvalidField("content file layout"));
        }
        verify_optional_digest(
            "svg_digest",
            &self.manifest.contents.svg_digest,
            self.manifest.contents.svg_ref.as_ref(),
            self.svg.as_deref(),
        )?;
        verify_optional_digest(
            "png_digest",
            &self.manifest.contents.png_digest,
            self.manifest.contents.png_ref.as_ref(),
            self.png.as_deref(),
        )?;
        verify_required_digest(
            "decision_digest",
            &self.manifest.contents.decision_digest,
            &self.manifest.contents.decision_ref,
            self.decision.as_deref(),
        )?;
        verify_required_digest(
            "viewer_digest",
            &self.manifest.contents.viewer_digest,
            &self.manifest.contents.viewer_ref,
            self.viewer.as_deref(),
        )?;
        validate_json_object(
            self.decision
                .as_deref()
                .ok_or(MapBundleError::DigestMismatch("decision_digest"))?,
            "decision json",
        )?;
        validate_json_object(
            self.viewer
                .as_deref()
                .ok_or(MapBundleError::DigestMismatch("viewer_digest"))?,
            "viewer json",
        )?;
        if self.manifest.map_instance != self.analysis.map_instance
            || self.manifest.act != self.analysis.act
            || self.manifest.history.source_state_id != self.analysis.source_state_id
            || self.manifest.history.generation != self.analysis.generation
        {
            return Err(MapBundleError::IdentityMismatch);
        }
        let bytes = self.canonical_manifest_bytes_without_digest()?;
        if bytes.len() > MAP_MAX_BUNDLE_BYTES {
            return Err(MapBundleError::TooLarge("manifest"));
        }
        Ok(())
    }

    fn validate_manifest(&self) -> Result<(), MapBundleError> {
        let fields = [
            &self.manifest.snapshot_digest,
            &self.manifest.analysis_digest,
            &self.manifest.map_instance,
            &self.manifest.act,
            &self.manifest.run_id,
            &self.manifest.episode_id,
            &self.manifest.trajectory_id,
            &self.manifest.schema_profile,
            &self.manifest.schema_digest,
            &self.manifest.analysis_version,
            &self.manifest.renderer_version,
            &self.manifest.origin.owner,
            &self.manifest.origin.source,
            &self.manifest.origin.generator,
            &self.manifest.origin.license,
            &self.manifest.history.source_state_id,
            &self.manifest.history.action_catalog_digest,
        ];
        if fields
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_BUNDLE_TEXT_BYTES)
        {
            return Err(MapBundleError::InvalidField("manifest text"));
        }
        if !is_digest(&self.manifest.schema_digest)
            || !is_digest(&self.manifest.snapshot_digest)
            || !is_digest(&self.manifest.analysis_digest)
            || !is_digest(&self.manifest.history.action_catalog_digest)
        {
            return Err(MapBundleError::InvalidDigest("manifest"));
        }
        if self.manifest.schema_profile == "runtime-map-v1"
            && self.manifest.schema_digest != RUNTIME_MAP_SCHEMA_DIGEST
        {
            return Err(MapBundleError::DigestMismatch("schema_digest"));
        }
        if self
            .manifest
            .model_execution_id
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > MAX_BUNDLE_TEXT_BYTES)
        {
            return Err(MapBundleError::InvalidField("model_execution_id"));
        }
        for reference in [
            &self.manifest.contents.snapshot_ref,
            &self.manifest.contents.analysis_ref,
            &self.manifest.contents.decision_ref,
            &self.manifest.contents.viewer_ref,
        ] {
            validate_file_reference(reference)?;
        }
        for reference in [
            self.manifest.contents.svg_ref.as_ref(),
            self.manifest.contents.png_ref.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_file_reference(reference)?;
        }
        Ok(())
    }

    fn canonical_manifest_bytes_without_digest(&self) -> Result<Vec<u8>, MapBundleError> {
        let mut manifest = self.manifest.clone();
        manifest.bundle_digest.clear();
        canonical_bytes(&manifest).map_err(MapBundleError::Canonical)
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
