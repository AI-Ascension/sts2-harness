// SPDX-License-Identifier: MIT

use super::*;

use std::error::Error;
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "sts2-private-retained-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    fn policy(&self, quota_bytes: u64) -> ExoPrivateStatePolicy {
        ExoPrivateStatePolicy {
            state_root: self.0.join("state").display().to_string(),
            cache_root: self.0.join("cache").display().to_string(),
            temp_root: self.0.join("temp").display().to_string(),
            quota_bytes,
            max_retention_days: 1,
            permissions_octal: 0o700,
        }
    }

    fn remove(self) -> io::Result<()> {
        fs::remove_dir_all(self.0)
    }
}

fn config_digest() -> String {
    crate::sha256_hex(b"retained-owner-fixture")
}

fn expired_timestamp(policy: &ExoPrivateStatePolicy) -> Result<u64, Box<dyn Error>> {
    let retention = u64::from(policy.max_retention_days) * 24 * 60 * 60;
    Ok(unix_seconds()
        .map_err(io::Error::other)?
        .checked_sub(retention + 1)
        .ok_or_else(|| io::Error::other("retention fixture clock too small"))?)
}

fn attempt_paths(run: &GuardedRun) -> [PathBuf; 3] {
    [
        run.paths.state_root.clone(),
        run.paths.cache_root.clone(),
        run.paths.temp_root.clone(),
    ]
}

fn assert_exact_markers(run: &GuardedRun) -> Result<(), Box<dyn Error>> {
    for (index, attempt) in run.attempts.iter().enumerate() {
        let bytes = read_marker(&attempt.file).map_err(io::Error::other)?;
        let marker: OwnerMarker = serde_json::from_slice(&bytes)?;
        assert_eq!(marker.schema, MARKER_SCHEMA);
        assert_eq!(marker.attempt_id, run.attempt_id);
        assert_eq!(marker.config_digest, run.config_digest);
        assert_eq!(marker.policy_digest, run.policy_digest);
        assert_eq!(marker.service_uid, rustix::process::geteuid().as_raw());
        assert_eq!(marker.root_kind, ROOT_KINDS[index]);
        assert_eq!(marker.root_identity, attempt.identity);
        assert_eq!(marker.phase, MarkerPhase::Quiescent);
        assert!(marker.process.is_none());
        assert_eq!(marker.root_proofs.len(), ROOT_KINDS.len());
        for (proof_index, proof) in marker.root_proofs.iter().enumerate() {
            assert_eq!(proof.kind, ROOT_KINDS[proof_index]);
            assert_eq!(proof.base_identity, run.roots[proof_index].identity);
            assert_eq!(proof.attempt_identity, run.attempts[proof_index].identity);
        }
    }
    Ok(())
}

fn update_markers(
    run: &mut GuardedRun,
    mut update: impl FnMut(&mut OwnerMarker),
) -> Result<(), Box<dyn Error>> {
    for attempt in &mut run.attempts {
        let bytes = read_marker(&attempt.file).map_err(io::Error::other)?;
        let mut marker: OwnerMarker = serde_json::from_slice(&bytes)?;
        update(&mut marker);
        let bytes = serde_json::to_vec(&marker)?;
        write_marker(&mut attempt.marker, &bytes).map_err(io::Error::other)?;
    }
    Ok(())
}

fn assert_attempt_only(
    policy: &ExoPrivateStatePolicy,
    expected_id: &str,
) -> Result<(), Box<dyn Error>> {
    for path in [
        PathBuf::from(&policy.state_root),
        PathBuf::from(&policy.cache_root),
        PathBuf::from(&policy.temp_root),
    ] {
        let root = open_policy_root(&path, false).map_err(io::Error::other)?;
        let names = policy_attempt_names(&root, true).map_err(io::Error::other)?;
        assert_eq!(names, vec![std::ffi::OsString::from(expected_id)]);
    }
    Ok(())
}

fn create_payloads(run: &GuardedRun, name: &str, bytes: u64) -> Result<(), Box<dyn Error>> {
    for attempt in &run.attempts {
        let payload = crate::exo_private_state::fs::create_private_file(
            &attempt.file,
            std::ffi::OsStr::new(name),
        )
        .map_err(io::Error::other)?;
        payload.set_len(bytes)?;
    }
    Ok(())
}

fn assert_payloads(paths: &[PathBuf; 3], name: &str, bytes: u64) -> io::Result<()> {
    for path in paths {
        assert_eq!(fs::metadata(path.join(name))?.len(), bytes);
    }
    Ok(())
}

#[test]
fn expired_quiescent_attempt_is_removed_but_starting_attempt_is_preserved()
-> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let policy = scratch.policy(256 * 1024);
    let digest = config_digest();
    let expired_paths;
    {
        let mut expired = GuardedRun::create(&policy, &digest).map_err(io::Error::other)?;
        // This uses the real marker writer to fixture matching quiescent records. Process
        // quiescence itself is covered separately by the bridge child lifecycle tests.
        expired.mark_quiescent(None).map_err(io::Error::other)?;
        assert_exact_markers(&expired)?;
        let created_at = expired_timestamp(&policy)?;
        update_markers(&mut expired, |marker| {
            marker.created_at_unix_seconds = created_at;
        })?;
        expired_paths = attempt_paths(&expired);
    }
    assert!(expired_paths.iter().all(|path| path.is_dir()));

    let mut admitted = GuardedRun::create(&policy, &digest).map_err(io::Error::other)?;
    assert!(expired_paths.iter().all(|path| !path.exists()));
    admitted.finish().map_err(io::Error::other)?;
    drop(admitted);

    let mut starting = GuardedRun::create(&policy, &digest).map_err(io::Error::other)?;
    starting.begin_spawn().map_err(io::Error::other)?;
    let created_at = expired_timestamp(&policy)?;
    update_markers(&mut starting, |marker| {
        marker.created_at_unix_seconds = created_at;
    })?;
    let starting_id = starting.attempt_id().to_owned();
    let starting_paths = attempt_paths(&starting);
    drop(starting);
    assert_eq!(
        GuardedRun::create(&policy, &digest).err(),
        Some("exo_private_retained_identity")
    );
    assert!(starting_paths.iter().all(|path| path.is_dir()));
    assert_attempt_only(&policy, &starting_id)?;
    scratch.remove()?;
    Ok(())
}

#[test]
fn foreign_policy_markers_are_refused_without_removing_owned_paths() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let policy = scratch.policy(256 * 1024);
    let digest = config_digest();
    let mut foreign = GuardedRun::create(&policy, &digest).map_err(io::Error::other)?;
    foreign.mark_quiescent(None).map_err(io::Error::other)?;
    assert_exact_markers(&foreign)?;
    let foreign_digest = "f".repeat(64);
    assert_ne!(foreign_digest, foreign.policy_digest);
    let created_at = expired_timestamp(&policy)?;
    update_markers(&mut foreign, |marker| {
        marker.policy_digest = foreign_digest.clone();
        marker.created_at_unix_seconds = created_at;
    })?;
    let attempt_id = foreign.attempt_id().to_owned();
    let paths = attempt_paths(&foreign);
    drop(foreign);

    assert_eq!(
        GuardedRun::create(&policy, &digest).err(),
        Some("exo_private_retained_identity")
    );
    assert!(paths.iter().all(|path| path.is_dir()));
    assert_attempt_only(&policy, &attempt_id)?;
    scratch.remove()?;
    Ok(())
}

#[test]
fn expired_attempt_with_a_missing_root_marker_refuses_and_preserves_payloads()
-> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let policy = scratch.policy(256 * 1024);
    let digest = config_digest();
    let mut retained = GuardedRun::create(&policy, &digest).map_err(io::Error::other)?;
    retained.mark_quiescent(None).map_err(io::Error::other)?;
    assert_exact_markers(&retained)?;
    let created_at = expired_timestamp(&policy)?;
    update_markers(&mut retained, |marker| {
        marker.created_at_unix_seconds = created_at;
    })?;
    create_payloads(&retained, "kept-after-marker-loss", 128)?;
    let attempt_id = retained.attempt_id().to_owned();
    let paths = attempt_paths(&retained);
    drop(retained);

    fs::remove_file(paths[1].join(crate::exo_private_state::OWNER_MARKER_NAME))?;
    assert_eq!(
        GuardedRun::create(&policy, &digest).err(),
        Some("exo_private_marker")
    );
    assert!(paths.iter().all(|path| path.is_dir()));
    assert_payloads(&paths, "kept-after-marker-loss", 128)?;
    assert_attempt_only(&policy, &attempt_id)?;
    scratch.remove()?;
    Ok(())
}

#[test]
fn retained_bytes_across_all_policy_roots_refuse_a_second_attempt() -> Result<(), Box<dyn Error>> {
    const QUOTA: u64 = 64 * 1024;
    const EACH_ROOT: u64 = 24 * 1024;

    let scratch = Scratch::new()?;
    let policy = scratch.policy(QUOTA);
    let digest = config_digest();
    let mut retained = GuardedRun::create(&policy, &digest).map_err(io::Error::other)?;
    retained.mark_quiescent(None).map_err(io::Error::other)?;
    assert_exact_markers(&retained)?;
    let attempt_id = retained.attempt_id().to_owned();
    let paths = attempt_paths(&retained);
    create_payloads(&retained, "retained-payload", EACH_ROOT)?;
    drop(retained);

    assert_eq!(
        GuardedRun::create(&policy, &digest).err(),
        Some("exo_private_quota")
    );
    assert_eq!(EACH_ROOT * 3, 72 * 1024);
    const {
        assert!(EACH_ROOT < QUOTA);
    }
    assert_payloads(&paths, "retained-payload", EACH_ROOT)?;
    assert_attempt_only(&policy, &attempt_id)?;
    scratch.remove()?;
    Ok(())
}
