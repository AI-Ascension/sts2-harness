// SPDX-License-Identifier: MIT

use super::PrivateSqliteGuard;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sts2-private-sqlite-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).expect("create temp root");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("make temp root private");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn private_child(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).expect("create child");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("make child private");
        path
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opens_private_store_and_revalidates_retained_inode() {
    let tree = TempTree::new();
    let parent = tree.private_child("store");
    let path = parent.join("memory.sqlite");
    let (connection, guard) =
        PrivateSqliteGuard::open(&path, 1024 * 1024).expect("private store opens");
    connection
        .execute_batch("CREATE TABLE guarded (value INTEGER);")
        .expect("sqlite remains usable");
    guard.verify().expect("retained store path remains valid");
}

#[test]
fn detects_replaced_database_after_private_open() {
    let tree = TempTree::new();
    let parent = tree.private_child("store");
    let path = parent.join("memory.sqlite");
    let (_connection, guard) =
        PrivateSqliteGuard::open(&path, 1024 * 1024).expect("private store opens");
    fs::rename(&path, parent.join("old.sqlite")).expect("move opened file");
    fs::write(&path, b"replacement").expect("replace store path");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("make replacement private");
    assert!(guard.verify().is_err());
}

#[test]
fn rejects_symlinked_leaf_and_parent_before_sqlite_open() {
    let tree = TempTree::new();
    let parent = tree.private_child("store");
    let target = parent.join("target.sqlite");
    fs::write(&target, b"unchanged").expect("create target");
    let leaf = parent.join("leaf.sqlite");
    symlink(&target, &leaf).expect("create leaf link");
    assert!(PrivateSqliteGuard::open(&leaf, 1024 * 1024).is_err());
    assert_eq!(fs::read(&target).expect("read target"), b"unchanged");

    let parent_alias = tree.path().join("store-link");
    symlink(&parent, &parent_alias).expect("create parent link");
    let through_link = parent_alias.join("new.sqlite");
    assert!(PrivateSqliteGuard::open(&through_link, 1024 * 1024).is_err());
    assert!(!parent.join("new.sqlite").exists());
}

#[test]
fn rejects_group_or_world_writable_ancestor() {
    let tree = TempTree::new();
    let writable = tree.path().join("writable");
    fs::create_dir(&writable).expect("create writable ancestor");
    fs::set_permissions(&writable, fs::Permissions::from_mode(0o777))
        .expect("make ancestor writable");
    let private = writable.join("private");
    fs::create_dir(&private).expect("create private child");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).expect("make child private");
    let path = private.join("new.sqlite");
    assert!(PrivateSqliteGuard::open(&path, 1024 * 1024).is_err());
    assert!(!path.exists());
}

#[test]
fn rejects_untrusted_existing_sidecar() {
    let tree = TempTree::new();
    let parent = tree.private_child("store");
    let path = parent.join("memory.sqlite");
    let target = parent.join("outside");
    fs::write(&target, b"unchanged").expect("create target");
    symlink(&target, parent.join("memory.sqlite-wal")).expect("create sidecar link");
    assert!(PrivateSqliteGuard::open(&path, 1024 * 1024).is_err());
    assert_eq!(fs::read(&target).expect("read target"), b"unchanged");
}
