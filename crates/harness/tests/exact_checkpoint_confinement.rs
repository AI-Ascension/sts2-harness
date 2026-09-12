// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sts2_harness::{BlobDigest, ExactArtifactStore, ExactCheckpointError, MAX_EXACT_BLOB_BYTES};

type TestResult = Result<(), Box<dyn std::error::Error>>;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Result<Self, std::io::Error> {
        let path = std::env::temp_dir().join(format!(
            "checkpoint-confinement-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn blob_path(root: &Path, digest: &BlobDigest) -> PathBuf {
    let hex = digest.as_str().trim_start_matches("sha256:");
    root.join("exact/blobs").join(&hex[..2]).join(hex)
}

#[test]
fn plain_publication_is_private_durable_deduplicated_and_bounded() -> TestResult {
    let workspace = Workspace::new()?;
    let store = ExactArtifactStore::new(workspace.0.join("new/root"));
    let digest = store.stage_blob(b"confined")?;
    assert_eq!(store.read_blob(&digest)?, b"confined");
    assert_eq!(store.stage_blob(b"confined")?, digest);
    let path = blob_path(store.root_directory(), &digest);
    assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
    let names =
        fs::read_dir(path.parent().ok_or("parent absent")?)?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        names.len(),
        1,
        "publication must not leave named staging files"
    );
    fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .set_len(MAX_EXACT_BLOB_BYTES as u64 + 1)?;
    assert_eq!(
        store.read_blob(&digest),
        Err(ExactCheckpointError::Oversized)
    );
    assert_eq!(
        store.stage_blob(b"confined"),
        Err(ExactCheckpointError::Oversized)
    );
    Ok(())
}

#[test]
fn symlink_blob_never_reads_or_rewrites_outside_target() -> TestResult {
    let workspace = Workspace::new()?;
    let store = ExactArtifactStore::new(workspace.0.join("store"));
    let digest = store.stage_blob(b"original")?;
    let path = blob_path(store.root_directory(), &digest);
    let outside = workspace.0.join("outside");
    fs::write(&outside, b"original")?;
    fs::remove_file(&path)?;
    symlink(&outside, &path)?;
    assert!(
        store.read_blob(&digest).is_err(),
        "matching bytes behind a symlink must be refused"
    );
    assert!(store.stage_blob(b"original").is_err());
    assert_eq!(fs::read(&outside)?, b"original");
    assert!(fs::symlink_metadata(&path)?.file_type().is_symlink());
    Ok(())
}

#[test]
fn every_parent_level_and_configured_root_refuses_symlink_escape() -> TestResult {
    for component in ["root", "exact", "blobs", "shard"] {
        let workspace = Workspace::new()?;
        let root = workspace.0.join("store");
        let store = ExactArtifactStore::new(&root);
        let digest = store.stage_blob(b"parent")?;
        let path = blob_path(&root, &digest);
        let attacked = match component {
            "root" => root.clone(),
            "exact" => root.join("exact"),
            "blobs" => root.join("exact/blobs"),
            _ => path.parent().ok_or("shard absent")?.to_owned(),
        };
        let outside = workspace.0.join("outside");
        fs::rename(&attacked, &outside)?;
        symlink(&outside, &attacked)?;
        assert!(store.read_blob(&digest).is_err(), "{component}");
        assert!(store.stage_blob(b"parent").is_err(), "{component}");
        if component != "shard" {
            assert!(store.stage_blob(b"new-write").is_err(), "{component}");
        }
    }
    Ok(())
}

#[test]
fn final_path_substitution_and_named_temp_traps_cannot_publish_symlink() -> TestResult {
    let workspace = Workspace::new()?;
    let root = workspace.0.join("store");
    let store = ExactArtifactStore::new(&root);
    let digest = store.stage_blob(b"race")?;
    let path = blob_path(&root, &digest);
    let outside = workspace.0.join("outside");
    fs::write(&outside, b"sentinel")?;
    let directory = path.parent().ok_or("shard absent")?;
    for nonce in 0..64 {
        symlink(
            &outside,
            directory.join(format!(".tmp-{}-{nonce}", std::process::id())),
        )?;
    }
    fs::remove_file(&path)?;
    let attacker_path = path.clone();
    let attacker_outside = outside.clone();
    let attacker = std::thread::spawn(move || {
        for _ in 0..256 {
            let _ = symlink(&attacker_outside, &attacker_path);
            let _ = fs::remove_file(&attacker_path);
        }
    });
    for _ in 0..256 {
        // A concurrent final-name replacement may cause a refusal or remove a published file.
        // Neither outcome is allowed to modify or read through the external symlink target.
        let _ = store.stage_blob(b"race");
        if let Ok(bytes) = store.read_blob(&digest) {
            assert_eq!(bytes, b"race");
        }
    }
    attacker.join().map_err(|_| "attacker thread failed")?;
    assert_eq!(fs::read(&outside)?, b"sentinel");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&path)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    assert_eq!(store.stage_blob(b"race")?, digest);
    assert_eq!(store.read_blob(&digest)?, b"race");
    assert!(fs::metadata(&path)?.is_file());
    Ok(())
}

#[test]
fn parent_traversal_and_nonregular_blob_are_refused() -> TestResult {
    let workspace = Workspace::new()?;
    let root = workspace.0.join("unused/../store");
    assert!(ExactArtifactStore::new(root).stage_blob(b"reject").is_err());
    let store = ExactArtifactStore::new(workspace.0.join("regular"));
    let digest = store.stage_blob(b"type")?;
    let path = blob_path(store.root_directory(), &digest);
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    assert!(store.read_blob(&digest).is_err());
    assert!(store.stage_blob(b"type").is_err());
    Ok(())
}
