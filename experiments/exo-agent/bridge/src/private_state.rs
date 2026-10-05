// SPDX-License-Identifier: MIT
//! Executor-side validation for the additive guarded-v2 private-state handoff.
//!
//! This module deliberately owns its checker. The isolated executor is not linked to the harness
//! implementation crate, so it independently opens and verifies the parent's durable ownership
//! records before `BasicExoHarness` can create SQLite state or bind a provider credential.

use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

// SPDX-License-Identifier: MIT

mod filesystem;
mod policy;
mod validate;

pub(crate) use validate::set_private_umask_and_validate;

use filesystem::{
    attempt_path, file_identity, open_private_root, path_for_kind, private_path_for_kind,
    read_marker, reject_root_aliases, same_shared_marker, validate_policy_path,
    verify_attempt_lock, verify_directory, verify_marker, verify_policy_lock,
};
use policy::{policy_digest, proc_identity, read_boot_id, valid_attempt_id, valid_digest};

const MARKER_NAME: &str = ".sts2-owner.json";
const POLICY_LOCK_NAME: &str = ".sts2-policy.lock";
const MARKER_SCHEMA: &str = "sts2.exo-private-state-owner-v2";
const POLICY_LOCK_RECORD: &str = "sts2.exo-private-policy-lock-v1";
const PRIVATE_STATE_VERSION: &str = "sts2.exo-private-state-executor-v2";
const MAX_MARKER_BYTES: usize = 16 * 1024;
const MAX_POLICY_LOCK_BYTES: usize = 256;
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const MAX_COMPONENTS: usize = 64;
const FORBIDDEN_ANY_COMPONENTS: [&str; 3] = ["home", "root", "users"];
const FORBIDDEN_FIRST_COMPONENTS: [&str; 9] = [
    "etc", "usr", "boot", "dev", "proc", "sys", "run", "media", "mnt",
];
const FORBIDDEN_GAME_MARKERS: [&str; 8] = [
    "slaythespire",
    "slaythespire2",
    "slay the spire 2",
    "steam",
    "steamapps",
    "steamlibrary",
    "steamuserdata",
    "saves",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    state_root: String,
    cache_root: String,
    temp_root: String,
    quota_bytes: u64,
    max_retention_days: u32,
    permissions_octal: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Paths {
    state_root: PathBuf,
    cache_root: PathBuf,
    temp_root: PathBuf,
    config_root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessIdentity {
    boot_id: String,
    pid: u32,
    parent_pid: u32,
    process_group: u32,
    session: u32,
    start_time_ticks: u64,
    uid: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct RootProof {
    kind: String,
    base_identity: FileIdentity,
    attempt_identity: FileIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum MarkerPhase {
    Preparing,
    Starting,
    Running,
    Quiescent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct OwnerMarker {
    schema: String,
    attempt_id: String,
    config_digest: String,
    policy_digest: String,
    service_uid: u32,
    created_at_unix_seconds: u64,
    boot_id: String,
    root_kind: String,
    root_proofs: Vec<RootProof>,
    root_identity: FileIdentity,
    phase: MarkerPhase,
    process: Option<ProcessIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrivateState {
    pub(crate) version: String,
    pub(crate) attempt_id: String,
    pub(crate) config_digest: String,
    pub(crate) policy_digest: String,
    pub(crate) policy: Policy,
    pub(crate) service_uid: u32,
    pub(crate) process: ProcessIdentity,
    pub(crate) paths: Paths,
}

struct OpenRoot {
    file: File,
    identity: FileIdentity,
    ancestors: Vec<FileIdentity>,
}

#[derive(Clone, Copy)]
struct ProcIdentity {
    parent_pid: u32,
    process_group: u32,
    session: u32,
    start_time_ticks: u64,
}

impl Policy {
    fn validate(&self) -> Result<(), &'static str> {
        if self.quota_bytes == 0
            || self.quota_bytes > 8 << 30
            || self.max_retention_days == 0
            || self.max_retention_days > 30
            || self.permissions_octal != 0o700
        {
            return Err("exo_private_policy");
        }
        for root in [&self.state_root, &self.cache_root, &self.temp_root] {
            validate_policy_path(Path::new(root))?;
        }
        let roots = [
            Path::new(&self.state_root),
            Path::new(&self.cache_root),
            Path::new(&self.temp_root),
        ];
        for left in 0..roots.len() {
            for right in (left + 1)..roots.len() {
                if roots[left] == roots[right]
                    || roots[left].starts_with(roots[right])
                    || roots[right].starts_with(roots[left])
                {
                    return Err("exo_private_policy");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_closed_policy_path_shapes_without_filesystem_side_effects() {
        let valid = Policy {
            state_root: "/var/lib/sts2/state".into(),
            cache_root: "/var/lib/sts2/cache".into(),
            temp_root: "/var/lib/sts2/temp".into(),
            quota_bytes: 1024,
            max_retention_days: 1,
            permissions_octal: 0o700,
        };
        assert!(valid.validate().is_ok());

        let mut traversal = valid.clone();
        traversal.temp_root = "/var/lib/sts2/../temp".into();
        assert!(traversal.validate().is_err());

        let mut overlap = valid.clone();
        overlap.cache_root = "/var/lib/sts2/state/cache".into();
        assert!(overlap.validate().is_err());

        let mut loose = valid;
        loose.permissions_octal = 0o777;
        assert!(loose.validate().is_err());
    }

    #[test]
    fn file_identity_contains_inode_and_permission_facts() {
        let file = File::open("/");
        assert!(file.is_ok());
        let Ok(file) = file else {
            return;
        };
        let identity = file_identity(&file);
        assert!(identity.is_ok());
        if let Ok(identity) = identity {
            assert!(identity.inode > 0);
            assert_ne!(identity.mode, 0);
        }
    }
}
