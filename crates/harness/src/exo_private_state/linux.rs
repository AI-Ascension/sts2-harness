// SPDX-License-Identifier: MIT

mod marker;
mod quota;
mod retained;
mod util;

use marker::{MARKER_SCHEMA, MarkerPhase, OwnerMarker, ROOT_KINDS, RootProof};
use quota::{reject_aliased_roots, scan_policy, verify_policy_locks};
use retained::reconcile_retained;
use util::{cleanup_partial, unix_seconds, valid_attempt_id, valid_boot_id, valid_digest};

use std::ffi::OsStr;
use std::fs::File;
use std::path::PathBuf;

use uuid::Uuid;

use crate::ExoPrivateStatePolicy;

use super::fs::{
    AttemptDirectory, MAX_POLICY_LOCK_BYTES, PolicyLock, PolicyRoot, create_attempt,
    create_private_child, lock_policy_roots, open_existing_attempt, open_policy_root,
    policy_attempt_names, read_marker, remove_attempt, scan_policy_root, scan_run, verify_attempt,
    verify_policy_lock, verify_policy_root, write_marker,
};
use super::process::{BridgeChildScope, boot_id};
use super::{EXECUTOR_STATE_VERSION, ExecutorPrivateState, RunPaths};

pub struct GuardedRun {
    policy: ExoPrivateStatePolicy,
    config_digest: String,
    policy_digest: String,
    attempt_id: String,
    created_at_unix_seconds: u64,
    boot_id: String,
    roots: Vec<PolicyRoot>,
    attempts: Vec<AttemptDirectory>,
    paths: RunPaths,
    _config_root: File,
    _policy_locks: Vec<PolicyLock>,
    process: Option<super::ProcessIdentity>,
    phase: MarkerPhase,
    markers_quiescent: bool,
    spawn_unknown: bool,
}

impl GuardedRun {
    pub fn create(
        policy: &ExoPrivateStatePolicy,
        config_digest: &str,
    ) -> Result<Self, &'static str> {
        policy.validate().map_err(|_| "exo_private_policy")?;
        if !valid_digest(config_digest) {
            return Err("exo_private_config_identity");
        }
        let policy_digest = policy.policy_digest();
        let roots = [
            PathBuf::from(&policy.state_root),
            PathBuf::from(&policy.cache_root),
            PathBuf::from(&policy.temp_root),
        ]
        .iter()
        .map(|path| open_policy_root(path, true))
        .collect::<Result<Vec<_>, _>>()?;
        reject_aliased_roots(&roots)?;
        let policy_locks = lock_policy_roots(&roots, &policy_digest)?;
        verify_policy_locks(&roots, &policy_locks)?;
        reconcile_retained(&roots, &policy_locks, policy, &policy_digest)?;
        let retained_bytes = scan_policy(&roots, &policy_locks, policy.quota_bytes)?;
        if retained_bytes > policy.quota_bytes {
            return Err("exo_private_quota");
        }

        let created_at_unix_seconds = unix_seconds()?;
        let current_boot_id = boot_id()?;
        let attempt_id = Uuid::new_v4().simple().to_string();
        let mut attempts = Vec::with_capacity(3);
        for root in &roots {
            match create_attempt(root, OsStr::new(&attempt_id)) {
                Ok(attempt) => attempts.push(attempt),
                Err(error) => {
                    return if cleanup_partial(&roots, &attempts).is_ok() {
                        Err(error)
                    } else {
                        Err("exo_private_partial_cleanup")
                    };
                }
            }
        }
        let config_root = match create_private_child(&attempts[1].file, OsStr::new("config")) {
            Ok(root) => root,
            Err(error) => {
                return if cleanup_partial(&roots, &attempts).is_ok() {
                    Err(error)
                } else {
                    Err("exo_private_partial_cleanup")
                };
            }
        };
        let paths = RunPaths {
            state_root: attempts[0].path.clone(),
            cache_root: attempts[1].path.clone(),
            temp_root: attempts[2].path.clone(),
            config_root: attempts[1].path.join("config"),
        };
        let mut run = Self {
            policy: policy.clone(),
            config_digest: config_digest.to_owned(),
            policy_digest,
            attempt_id,
            created_at_unix_seconds,
            boot_id: current_boot_id,
            roots,
            attempts,
            paths,
            _config_root: config_root,
            _policy_locks: policy_locks,
            process: None,
            phase: MarkerPhase::Preparing,
            markers_quiescent: false,
            spawn_unknown: false,
        };
        let initialized = run
            .write_markers(None, MarkerPhase::Preparing)
            .and_then(|()| {
                run.verify_quota()?;
                Ok(())
            });
        if let Err(error) = initialized {
            return if run.finish().is_ok() {
                Err(error)
            } else {
                Err("exo_private_initialization_cleanup")
            };
        }
        Ok(run)
    }

    pub const fn paths(&self) -> &RunPaths {
        &self.paths
    }

    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }

    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    /// Marks the spawn boundary before calling `Command::spawn`; an error must be followed by
    /// `spawn_failed`, while a successful spawn must be registered before writing its stdin.
    pub fn begin_spawn(&mut self) -> Result<(), &'static str> {
        if self.phase != MarkerPhase::Preparing || self.spawn_unknown {
            return Err("exo_private_spawn_phase");
        }
        self.write_markers(None, MarkerPhase::Starting)?;
        self.phase = MarkerPhase::Starting;
        self.spawn_unknown = true;
        Ok(())
    }

    pub fn spawn_failed(&mut self, child_scope: &BridgeChildScope) -> Result<(), &'static str> {
        if self.phase != MarkerPhase::Starting || !self.spawn_unknown {
            return Err("exo_private_spawn_phase");
        }
        child_scope.confirm_spawn_error()?;
        self.mark_quiescent(None)
    }

    pub fn record_child(&mut self, pid: u32) -> Result<ExecutorPrivateState, &'static str> {
        if self.phase != MarkerPhase::Starting || !self.spawn_unknown || self.process.is_some() {
            return Err("exo_private_spawn_phase");
        }
        self.spawn_unknown = true;
        let process = super::ProcessIdentity::read(pid)?;
        if process.uid != rustix::process::geteuid().as_raw() {
            return Err("exo_private_process_owner");
        }
        self.process = Some(process.clone());
        self.write_markers(Some(process.clone()), MarkerPhase::Running)?;
        self.phase = MarkerPhase::Running;
        self.spawn_unknown = false;
        Ok(ExecutorPrivateState {
            version: EXECUTOR_STATE_VERSION.to_owned(),
            attempt_id: self.attempt_id.clone(),
            config_digest: self.config_digest.clone(),
            policy_digest: self.policy_digest.clone(),
            policy: self.policy.clone(),
            service_uid: process.uid,
            process,
            paths: self.paths.clone(),
        })
    }

    pub fn verify_quota(&self) -> Result<u64, &'static str> {
        self.verify_owned_paths()?;
        let retained = scan_policy(&self.roots, &self._policy_locks, self.policy.quota_bytes)?;
        let current = scan_run(&self.attempts, self.policy.quota_bytes)?;
        if retained > self.policy.quota_bytes || current > self.policy.quota_bytes {
            return Err("exo_private_quota");
        }
        Ok(retained)
    }

    /// Persists quiescence only after the direct child has been waited and the one-shot bridge
    /// subreaper has drained/reaped every adopted descendant. `true` means no descendant existed;
    /// `false` means an unexpected descendant was killed, so the caller must withhold success.
    pub fn finish_child(&mut self, child_scope: &BridgeChildScope) -> Result<bool, &'static str> {
        if !matches!(self.phase, MarkerPhase::Starting | MarkerPhase::Running) {
            return Err("exo_private_spawn_phase");
        }
        let no_descendants = child_scope.drain()?;
        self.verify_quota()?;
        self.mark_quiescent(self.process.clone())?;
        Ok(no_descendants)
    }

    /// Explicit cleanup only. There is intentionally no Drop cleanup path.
    pub fn finish(&mut self) -> Result<(), &'static str> {
        if self.spawn_unknown {
            return Err("exo_private_process_ambiguous");
        }
        if self.phase == MarkerPhase::Preparing {
            // No spawn boundary was crossed in this live GuardedRun. A crash in Preparing remains
            // intentionally unreconciled; only an explicit successful update to Quiescent permits
            // a later process to clean it.
            self.mark_quiescent(None)?;
        }
        if self.phase != MarkerPhase::Quiescent || !self.markers_quiescent {
            return Err("exo_private_process_ambiguous");
        }
        self.verify_owned_paths()?;
        verify_policy_locks(&self.roots, &self._policy_locks)?;
        for index in 0..self.attempts.len() {
            remove_attempt(&self.roots[index], &self.attempts[index], u64::MAX)?;
        }
        Ok(())
    }

    fn verify_owned_paths(&self) -> Result<(), &'static str> {
        for (root, attempt) in self.roots.iter().zip(&self.attempts) {
            verify_policy_root(root)?;
            verify_attempt(root, attempt)?;
        }
        Ok(())
    }

    fn write_markers(
        &mut self,
        process: Option<super::ProcessIdentity>,
        phase: MarkerPhase,
    ) -> Result<(), &'static str> {
        self.markers_quiescent = false;
        let root_proofs = self
            .roots
            .iter()
            .zip(&self.attempts)
            .zip(ROOT_KINDS)
            .map(|((root, attempt), kind)| RootProof {
                kind: kind.to_owned(),
                base_identity: root.identity.clone(),
                attempt_identity: attempt.identity.clone(),
            })
            .collect::<Vec<_>>();
        for (index, attempt) in self.attempts.iter_mut().enumerate() {
            let marker = OwnerMarker {
                schema: MARKER_SCHEMA.to_owned(),
                attempt_id: self.attempt_id.clone(),
                config_digest: self.config_digest.clone(),
                policy_digest: self.policy_digest.clone(),
                service_uid: rustix::process::geteuid().as_raw(),
                created_at_unix_seconds: self.created_at_unix_seconds,
                boot_id: self.boot_id.clone(),
                root_kind: ROOT_KINDS[index].to_owned(),
                root_proofs: root_proofs.clone(),
                root_identity: attempt.identity.clone(),
                phase,
                process: process.clone(),
            };
            write_marker(
                &mut attempt.marker,
                &serde_json::to_vec(&marker).map_err(|_| "exo_private_marker_write")?,
            )?;
        }
        self.phase = phase;
        self.markers_quiescent = phase == MarkerPhase::Quiescent;
        Ok(())
    }

    fn mark_quiescent(
        &mut self,
        process: Option<super::ProcessIdentity>,
    ) -> Result<(), &'static str> {
        self.write_markers(process, MarkerPhase::Quiescent)?;
        self.spawn_unknown = false;
        Ok(())
    }
}
