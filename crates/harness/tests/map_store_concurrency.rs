// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sts2_harness::{
    BundleFileStore, MAP_MAX_FEED_ENTRIES, MapBundleError, MapBundleFeed, MapViewBundle,
    RuntimeMapBundleIdentity,
};

const PUBLICATION_LOCK_FILE: &str = ".sts2-map-publication.lock";

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/map-bundle-v1")
}

fn fixture_bundle() -> MapViewBundle {
    let store = BundleFileStore::new(fixture_root()).expect("fixture root");
    let feed = store.read_feed().expect("fixture feed");
    store
        .load(feed.head_digest().expect("fixture head"))
        .expect("fixture bundle")
}

fn bundle_for(template: &MapViewBundle, index: usize) -> MapViewBundle {
    MapViewBundle::from_runtime_snapshot(
        template.snapshot_bytes.clone(),
        RuntimeMapBundleIdentity {
            run_id: format!("concurrency-run-{index}"),
            episode_id: format!("concurrency-episode-{index}"),
            trajectory_id: format!("concurrency-trajectory-{index}"),
            model_execution_id: None,
            action_catalog_digest: format!("{index:064x}"),
        },
    )
    .expect("concurrency bundle")
}

fn temporary_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sts2-map-store-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn publish_with_busy_retry(
    store: &BundleFileStore,
    bundle: &MapViewBundle,
    operation_id: &str,
    observed_at_unix_ms: u64,
) -> Result<sts2_harness::PublicationReceipt, MapBundleError> {
    for _ in 0..64 {
        match store.publish_at(bundle, operation_id, observed_at_unix_ms) {
            Err(MapBundleError::Storage(message)) if message == "bundle publication lock busy" => {
                thread::sleep(Duration::from_millis(5));
            }
            result => return result,
        }
    }
    Err(MapBundleError::Storage(
        "bundle publication lock busy".to_owned(),
    ))
}

#[test]
fn concurrent_distinct_publications_preserve_every_acknowledged_feed_entry() {
    let root = temporary_root("concurrency");
    let template = fixture_bundle();
    let count = 24;
    let bundles = (0..count)
        .map(|index| bundle_for(&template, index + 1))
        .collect::<Vec<_>>();
    let barrier = Arc::new(Barrier::new(count));
    let handles = bundles
        .into_iter()
        .enumerate()
        .map(|(index, bundle)| {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let store = BundleFileStore::new(root).expect("store");
                publish_with_busy_retry(
                    &store,
                    &bundle,
                    &format!("concurrent-{index}"),
                    index as u64 + 1,
                )
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("publisher thread"))
        .collect::<Result<Vec<_>, _>>();
    let store = BundleFileStore::new(&root).expect("final store");
    let feed = store.read_feed().expect("final feed");
    assert_eq!(
        results.expect("every publication acknowledged").len(),
        count
    );
    assert_eq!(feed.entries.len(), count);
    assert_eq!(feed.sequence, count as u64);
    assert_eq!(store.list().expect("published bundles").len(), count);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn publication_fails_closed_when_the_exclusive_lock_is_busy() {
    let root = temporary_root("busy");
    let store = BundleFileStore::new(&root).expect("store");
    let bundle = fixture_bundle();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(PUBLICATION_LOCK_FILE))
        .expect("lock file");
    lock.lock().expect("hold publication lock");
    let started = Instant::now();
    let result = store.publish_at(&bundle, "busy-operation", 1);
    assert!(matches!(
        result,
        Err(MapBundleError::Storage(message)) if message == "bundle publication lock busy"
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(lock);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn an_unlocked_persistent_lock_file_is_reusable_after_writer_exit() {
    let root = temporary_root("stale-lock");
    let store = BundleFileStore::new(&root).expect("store");
    let bundle = fixture_bundle();
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(PUBLICATION_LOCK_FILE))
        .expect("persistent lock file");
    let receipt = store
        .publish_at(&bundle, "stale-lock-operation", 1)
        .expect("unlocked persistent file is reusable");
    assert_eq!(receipt.bundle_digest(), bundle.bundle_digest());
    assert!(root.join(PUBLICATION_LOCK_FILE).is_file());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn reused_operation_temp_directory_is_never_removed_or_overwritten() {
    let root = temporary_root("collision");
    let store = BundleFileStore::new(&root).expect("store");
    let bundle = fixture_bundle();
    let temporary = root.join(format!(
        ".{}.collision-operation.tmp",
        bundle.bundle_digest()
    ));
    fs::create_dir(&temporary).expect("preexisting temporary directory");
    let result = store.publish_at(&bundle, "collision-operation", 1);
    assert!(matches!(result, Err(MapBundleError::Storage(_))));
    assert!(temporary.is_dir());
    assert!(!root.join(bundle.bundle_digest()).exists());
    fs::remove_dir_all(root).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn feed_temp_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let root = temporary_root("symlink");
    let store = BundleFileStore::new(&root).expect("store");
    let bundle = fixture_bundle();
    let target = root.join("outside-target");
    fs::write(&target, b"sentinel").expect("target");
    let temporary = root.join(".feed.json.symlink-operation.tmp");
    symlink(&target, &temporary).expect("temporary symlink");
    let result = store.publish_at(&bundle, "symlink-operation", 1);
    assert!(matches!(result, Err(MapBundleError::Storage(_))));
    assert!(temporary.is_symlink());
    assert_eq!(fs::read(&target).expect("target bytes"), b"sentinel");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn stored_bundle_capacity_is_finite_and_does_not_delete_history() {
    let root = temporary_root("capacity");
    let store = BundleFileStore::new(&root).expect("store");
    for index in 0..MAP_MAX_FEED_ENTRIES {
        fs::create_dir(root.join(format!("{index:064x}"))).expect("bundle directory");
    }
    let bundle = fixture_bundle();
    let result = store.publish_at(&bundle, "capacity-operation", 1);
    assert!(matches!(
        result,
        Err(MapBundleError::TooLarge("bundle store capacity"))
    ));
    assert_eq!(
        store.list().expect("retained bundles").len(),
        MAP_MAX_FEED_ENTRIES
    );
    fs::remove_dir_all(root).expect("cleanup");
}
