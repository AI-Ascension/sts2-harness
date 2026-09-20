// SPDX-License-Identifier: MIT

use super::*;
use serde_json::json;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::sync::atomic::{AtomicU64, Ordering};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/jev-capture-store-tests");
        std::fs::create_dir_all(&base).expect("parent");
        let path = base.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("directory");
        Self(std::fs::canonicalize(path).expect("canonical"))
    }

    fn path(&self) -> &str {
        self.0.to_str().expect("path")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn capture_store_is_create_only_and_private() {
    let root = Scratch::new();
    let reserved =
        Reservation::reserve(root.path(), &json!({"status": "pending"}), 2).expect("reserve");
    reserved
        .finish(&json!({"status": "complete"}))
        .expect("finish");
    assert!(reserved.finish(&json!({"changed": true})).is_err());
    for suffix in ["pending", "result"] {
        let metadata = std::fs::metadata(root.0.join(format!("attempt-0000.{suffix}.json")))
            .expect("metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn quota_counts_incomplete_reservations_and_never_recycles_them() {
    let root = Scratch::new();
    let _ = Reservation::reserve(root.path(), &json!({"status": "pending"}), 2).expect("first");
    let _ = Reservation::reserve(root.path(), &json!({"status": "pending"}), 2).expect("second");
    assert!(matches!(
        Reservation::reserve(root.path(), &json!({}), 2),
        Err(Error::Quota)
    ));
}

#[test]
fn oversize_is_rejected_without_reserving_a_slot() {
    let root = Scratch::new();
    assert!(Reservation::reserve(root.path(), &json!("x".repeat(MAX_RECORD_BYTES)), 2).is_err());
    assert_eq!(std::fs::read_dir(&root.0).expect("entries").count(), 0);
}

#[test]
fn symlink_directory_and_occupied_final_are_refused() {
    let root = Scratch::new();
    let link = root.0.join("alias");
    std::os::unix::fs::symlink(&root.0, &link).expect("link");
    assert!(Reservation::reserve(link.to_str().expect("path"), &json!({}), 2).is_err());
    std::fs::write(root.0.join("attempt-0000.result.json"), b"untouched").expect("occupied");
    assert!(Reservation::reserve(root.path(), &json!({}), 2).is_err());
    assert_eq!(
        std::fs::read(root.0.join("attempt-0000.result.json")).expect("original"),
        b"untouched"
    );
}

#[test]
fn concurrent_reservations_never_share_a_slot() {
    let root = Scratch::new();
    let paths = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    Reservation::reserve(root.path(), &json!({"status": "pending"}), 8)
                        .expect("reserve")
                        .slot
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("joined"))
            .collect::<Vec<_>>()
    });
    let unique = paths.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), 8);
}
