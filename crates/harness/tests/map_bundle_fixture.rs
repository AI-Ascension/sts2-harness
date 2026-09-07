// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use sts2_harness::{BundleFileStore, HistoricalReplay, MapBundleFeed};

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

    let replay = HistoricalReplay::from_bundle(&bundle);
    assert_eq!(replay.bundle_digest, digest);
    assert!(replay.bindings.iter().all(|binding| !binding.dispatchable));
    assert!(replay
        .bindings
        .iter()
        .all(|binding| binding.generation == bundle.manifest.history.generation));
}
