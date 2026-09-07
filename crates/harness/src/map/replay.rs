// SPDX-License-Identifier: MIT

use super::bundle::{BundleManifest, MapBundleError, MapViewBundle};
use sts2_protocol::decode_map_snapshot;

/// One source-time legal action retained for replay inspection.
///
/// A historical binding deliberately carries no dispatch capability. The action and node IDs
/// identify the source record only; dispatch requires a fresh host catalog through the episode
/// runtime boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalActionBinding {
    pub node_id: String,
    pub action_id: String,
    pub generation: u64,
    pub dispatchable: bool,
}

/// An immutable, source-time view of a published map bundle.
///
/// Construction validates the complete bundle before copying any replay state. The source
/// snapshot, analysis, manifest, and all legal bindings remain available even if a later bundle
/// has a different map, generation, catalog, or analysis projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalReplay {
    pub bundle_digest: String,
    pub generation: u64,
    pub source_snapshot: Vec<u8>,
    pub analysis: super::analysis::MapAnalysis,
    pub manifest: BundleManifest,
    pub bindings: Vec<HistoricalActionBinding>,
}

impl HistoricalReplay {
    /// Loads and validates one complete source-time bundle without contacting a provider or game.
    pub fn from_bundle(bundle: &MapViewBundle) -> Result<Self, MapBundleError> {
        bundle.validate()?;
        if bundle.manifest.analysis_version != bundle.analysis.analysis_version {
            return Err(MapBundleError::IdentityMismatch);
        }
        let snapshot = decode_map_snapshot(&bundle.snapshot_bytes)
            .map_err(|error| MapBundleError::ProtocolSnapshot(error.to_string()))?;
        let generation = bundle.manifest.history.generation;
        let bindings = snapshot
            .bindings
            .into_iter()
            .map(|binding| HistoricalActionBinding {
                node_id: binding.graph_node_id,
                action_id: binding.host_action_id,
                generation,
                dispatchable: false,
            })
            .collect();
        Ok(Self {
            bundle_digest: bundle.bundle_digest().to_owned(),
            generation,
            source_snapshot: bundle.snapshot_bytes.clone(),
            analysis: bundle.analysis.clone(),
            manifest: bundle.manifest.clone(),
            bindings,
        })
    }

    /// Returns the source-state identity retained in the historical manifest.
    #[must_use]
    pub fn source_state_id(&self) -> &str {
        &self.manifest.history.source_state_id
    }

    /// Returns the source-time action catalog identity retained in the historical manifest.
    #[must_use]
    pub fn action_catalog_digest(&self) -> &str {
        &self.manifest.history.action_catalog_digest
    }

    /// Returns the analysis version that produced the retained source analysis.
    #[must_use]
    pub fn analysis_version(&self) -> &str {
        &self.manifest.analysis_version
    }

    /// Returns the exact source snapshot bytes validated at construction.
    #[must_use]
    pub fn source_snapshot_bytes(&self) -> &[u8] {
        &self.source_snapshot
    }

    /// Returns the complete source-time manifest, including independent lineage identities.
    #[must_use]
    pub fn source_manifest(&self) -> &BundleManifest {
        &self.manifest
    }

    /// Historical bindings are records for inspection and can never be dispatched.
    #[must_use]
    pub fn dispatchable_bindings(&self) -> impl Iterator<Item = &HistoricalActionBinding> {
        let _ = self;
        std::iter::empty()
    }
}
