// SPDX-License-Identifier: MIT

//! Immutable workflow identity used by the durable runtime-v3 gameplay admission boundary.
//!
//! The workflow is part of the execution identity because a combat demonstration and a complete
//! episode have different terminal semantics. Replay mode and the bytes that supplied a replay
//! are included for the same reason: a persisted checkpoint must never be resumed with a
//! different source or completion contract.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::EpisodeStage;

pub(super) const BINDING_VERSION: &str = "runtime-v3-workflow-binding-v1";
pub(super) const MAX_REPLAY_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkflowKind {
    FullEpisode,
    CombatDemo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplayKind {
    None,
    Full,
    Prefix,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WorkflowBinding {
    workflow: WorkflowKind,
    replay: ReplayKind,
    source_sha256: Option<String>,
}

impl WorkflowBinding {
    pub(super) fn for_launch(
        combat_demo: bool,
        replay_prefix: bool,
        replay_source: Option<&[u8]>,
    ) -> Result<Self, String> {
        let workflow = if combat_demo {
            WorkflowKind::CombatDemo
        } else {
            WorkflowKind::FullEpisode
        };
        let source_sha256 = replay_source.map(sha256_bytes);
        let replay = match source_sha256.is_some() {
            false => ReplayKind::None,
            true if replay_prefix && workflow == WorkflowKind::FullEpisode => ReplayKind::Prefix,
            true => ReplayKind::Full,
        };
        if replay != ReplayKind::None && source_sha256.is_none() {
            return Err(String::from(
                "runtime-v3 replay binding requires captured source bytes",
            ));
        }
        Ok(Self {
            workflow,
            replay,
            source_sha256,
        })
    }

    pub(super) fn default_full_episode() -> Self {
        Self {
            workflow: WorkflowKind::FullEpisode,
            replay: ReplayKind::None,
            source_sha256: None,
        }
    }

    pub(super) const fn is_combat_demo(&self) -> bool {
        matches!(self.workflow, WorkflowKind::CombatDemo)
    }

    pub(super) fn descriptor(&self) -> Value {
        json!({
            "version": BINDING_VERSION,
            "workflow": match self.workflow {
                WorkflowKind::FullEpisode => "full_episode",
                WorkflowKind::CombatDemo => "combat_demo",
            },
            "replay": match self.replay {
                ReplayKind::None => "none",
                ReplayKind::Full => "full",
                ReplayKind::Prefix => "prefix",
            },
            "source_sha256": self.source_sha256,
        })
    }
}

pub(super) fn completion_stage_allowed(stage: EpisodeStage, combat_demo: bool) -> bool {
    stage.is_terminal() || (combat_demo && stage == EpisodeStage::Reward)
}

pub(super) fn read_replay_source(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|_| String::from("cannot open replay trajectory"))?;
    let mut bytes = Vec::new();
    file.take(MAX_REPLAY_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| String::from("cannot read replay trajectory"))?;
    if bytes.len() as u64 > MAX_REPLAY_SOURCE_BYTES {
        return Err(String::from("replay trajectory exceeds byte bound"));
    }
    Ok(bytes)
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "runtime_v3_workflow_binding_tests.rs"]
mod tests;
