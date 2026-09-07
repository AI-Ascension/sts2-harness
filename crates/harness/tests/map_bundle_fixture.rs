// SPDX-License-Identifier: MIT

use std::path::PathBuf;
use std::{fs, process};

use sha2::{Digest as _, Sha256};
use sts2_harness::{
    AnalysisConfig, BundleFileStore, BundlePresentation, HistoricalReplay, MapAnalysis,
    MapBundleError, MapBundleFeed, MapViewBundle, RuntimeMapBundleIdentity, ValidatedMapGraph,
};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/map-bundle-v1")
}

#[test]
fn finalized_protocol_fixture_loads_from_feed_and_replays_without_dispatch() {
    let store = BundleFileStore::new(fixture_root()).expect("fixture root");
    let feed = store.read_feed().expect("operational feed");
    assert_eq!(feed.sequence, 1);
    assert_eq!(feed.entries.len(), 1);
    let digest = feed.head_digest().expect("feed head");
    let bundle = store.load(digest).expect("bundle files");
    assert_eq!(bundle.bundle_digest(), digest);
    assert_eq!(bundle.manifest.renderer_version, "unrendered");
    assert_eq!(bundle.viewer.as_deref(), Some(b"{}".as_slice()));
    assert_eq!(store.list().expect("feed list"), vec![digest.to_owned()]);
    let graph_digest = format!("{:x}", Sha256::digest(&bundle.snapshot_bytes));
    let graph = ValidatedMapGraph::from_visible_map_json(graph_digest, &bundle.snapshot_bytes)
        .expect("finalized visible-map adapter");
    assert_eq!(graph.map_instance(), "map-instance-1");
    assert_eq!(graph.act(), "1");
    assert_eq!(graph.legal_destinations().len(), 2);

    let regenerated_analysis = MapAnalysis::analyze(&graph, AnalysisConfig::default())
        .expect("deterministic fixture analysis");
    assert_eq!(
        bundle.analysis_bytes().expect("analysis bytes"),
        regenerated_analysis
            .canonical_bytes()
            .expect("regenerated analysis bytes")
    );
    let fixture_directory = fixture_root().join(digest);
    assert_eq!(
        fs::read(fixture_directory.join("analysis.json")).expect("golden analysis bytes"),
        regenerated_analysis
            .canonical_bytes()
            .expect("golden regenerated analysis bytes")
    );
    let regenerated = MapViewBundle::new(
        bundle.manifest.clone(),
        bundle.snapshot_bytes.clone(),
        regenerated_analysis,
        bundle.svg.clone(),
        bundle.png.clone(),
        bundle.decision.clone(),
        bundle.viewer.clone(),
    )
    .expect("regenerated fixture bundle");
    assert_eq!(regenerated.bundle_digest(), digest);
    assert_eq!(regenerated.manifest, bundle.manifest);
    assert_eq!(
        fs::read(fixture_directory.join("manifest.json")).expect("golden manifest bytes"),
        regenerated
            .canonical_manifest_bytes()
            .expect("golden regenerated manifest bytes")
    );

    let runtime_bundle = MapViewBundle::from_runtime_snapshot(
        bundle.snapshot_bytes.clone(),
        RuntimeMapBundleIdentity {
            run_id: bundle.manifest.run_id.clone(),
            episode_id: bundle.manifest.episode_id.clone(),
            trajectory_id: bundle.manifest.trajectory_id.clone(),
            model_execution_id: bundle.manifest.model_execution_id.clone(),
            action_catalog_digest: bundle.manifest.history.action_catalog_digest.clone(),
        },
    )
    .expect("runtime bundle builder");
    assert_eq!(runtime_bundle.manifest.renderer_version, "unrendered");
    assert_eq!(runtime_bundle.manifest.presentation, None);
    assert_eq!(runtime_bundle.viewer.as_deref(), Some(b"{}".as_slice()));

    let analysis_bytes = bundle.analysis_bytes().expect("analysis bytes");
    let invalid_decision = br#""not-an-object""#.to_vec();
    let mut invalid_decision_manifest = bundle.manifest.clone();
    invalid_decision_manifest.contents.decision_digest =
        Some(format!("{:x}", Sha256::digest(&invalid_decision)));
    assert!(matches!(
        MapViewBundle::from_manifest_and_files(
            invalid_decision_manifest,
            bundle.snapshot_bytes.clone(),
            &analysis_bytes,
            None,
            None,
            Some(invalid_decision),
            bundle.viewer.clone(),
        ),
        Err(MapBundleError::InvalidField("decision json"))
    ));

    let duplicate_viewer = br#"{"x":1,"x":2}"#.to_vec();
    let mut duplicate_viewer_manifest = bundle.manifest.clone();
    duplicate_viewer_manifest.renderer_version = "renderer-v1".to_owned();
    duplicate_viewer_manifest.presentation = Some(BundlePresentation {
        width: 320,
        height: 320,
        layout_version: "ascension-map-logical-v1".to_owned(),
    });
    duplicate_viewer_manifest.contents.viewer_digest =
        Some(format!("{:x}", Sha256::digest(&duplicate_viewer)));
    assert!(matches!(
        MapViewBundle::from_manifest_and_files(
            duplicate_viewer_manifest,
            bundle.snapshot_bytes.clone(),
            &analysis_bytes,
            None,
            None,
            bundle.decision.clone(),
            Some(duplicate_viewer),
        ),
        Err(MapBundleError::Canonical(_))
    ));

    let replay = HistoricalReplay::from_bundle(&bundle);
    assert_eq!(replay.bundle_digest, digest);
    assert!(replay.bindings.iter().all(|binding| !binding.dispatchable));
    assert!(
        replay
            .bindings
            .iter()
            .all(|binding| binding.generation == bundle.manifest.history.generation)
    );

    let temporary_root =
        std::env::temp_dir().join(format!("sts2-map-bundle-test-{}", process::id()));
    let _ = fs::remove_dir_all(&temporary_root);
    let temporary_store = BundleFileStore::new(&temporary_root).expect("temporary store");
    let receipt = temporary_store
        .publish_at(&bundle, "fixture-publish", 1_700_000_000_001)
        .expect("atomic publication");
    assert!(!receipt.already_present());
    assert_eq!(
        temporary_store.read_feed().expect("new feed").head_digest(),
        Some(digest)
    );
    let retry = temporary_store
        .publish_at(&bundle, "fixture-retry", 1_700_000_000_002)
        .expect("idempotent retry");
    assert!(retry.already_present());
    assert_eq!(
        temporary_store.read_feed().expect("stable feed").sequence,
        1
    );
    fs::remove_dir_all(temporary_root).expect("cleanup");
}
