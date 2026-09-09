// SPDX-License-Identifier: MIT

use super::{BINDING_VERSION, WorkflowBinding, completion_stage_allowed};
use serde_json::json;
use sha2::{Digest, Sha256};
use sts2_harness::{
    EpisodeObservation, EpisodeStage, ExecutionFingerprint, ExecutionLineage, ExecutionStore,
    ResumeState,
};

use super::super::durable::DurableHandle;

fn fingerprint(binding: &WorkflowBinding) -> Result<ExecutionFingerprint, String> {
    let bytes = serde_json::to_vec(&binding.descriptor()).map_err(|error| error.to_string())?;
    ExecutionFingerprint::new(
        "synthetic-seed",
        "synthetic-build",
        "synthetic-state",
        format!("{:x}", Sha256::digest(bytes)),
        "synthetic-provider",
    )
    .map_err(|error| error.to_string())
}

fn lineage() -> Result<ExecutionLineage, String> {
    ExecutionLineage::new(
        "workflow-binding-run",
        "workflow-binding-episode",
        "workflow-binding-attempt",
        "workflow-binding-trajectory",
    )
    .map_err(|error| error.to_string())
}

fn reward_observation() -> Result<EpisodeObservation, String> {
    EpisodeObservation::new(
        "reward-state",
        1,
        EpisodeStage::Reward,
        false,
        false,
        false,
        json!({
            "state_id": "reward-state",
            "generation": 1,
            "visible_seed": "synthetic-seed",
            "player": {
                "hp": 50,
                "max_hp": 50,
                "energy": 3,
                "gold": 99,
                "hand": [],
                "deck": [],
                "discard": [],
                "exhaust": []
            },
            "state": {"state": "reward", "options": []},
            "legal_actions": []
        }),
    )
    .map_err(|error| error.to_string())
}

#[test]
fn workflow_and_replay_choices_are_distinct_bindings() -> Result<(), String> {
    let source = br#"{"event":"source"}
"#;
    let full = WorkflowBinding::for_launch(false, false, Some(source))?;
    let prefix = WorkflowBinding::for_launch(false, true, Some(source))?;
    let combat = WorkflowBinding::for_launch(true, false, Some(source))?;
    let other_source = WorkflowBinding::for_launch(
        false,
        false,
        Some(
            br#"{"event":"other"}
"#,
        ),
    )?;
    assert_ne!(full, prefix);
    assert_ne!(full, combat);
    assert_ne!(full, other_source);
    assert_eq!(full.descriptor()["version"], BINDING_VERSION);
    assert_eq!(full.descriptor()["replay"], "full");
    assert!(full.descriptor()["source_sha256"].is_string());
    Ok(())
}

#[test]
fn absent_source_is_unreplayed_and_prefix_does_not_enable_combat_reward_mode() -> Result<(), String>
{
    let binding = WorkflowBinding::for_launch(false, false, None)?;
    assert_eq!(
        binding.descriptor(),
        json!({
            "version": BINDING_VERSION,
            "workflow": "full_episode",
            "replay": "none",
            "source_sha256": null,
        })
    );
    assert!(!completion_stage_allowed(EpisodeStage::Reward, false));
    assert!(completion_stage_allowed(EpisodeStage::Reward, true));
    assert!(completion_stage_allowed(EpisodeStage::Victory, false));
    Ok(())
}

#[test]
fn bounded_replay_source_read_rejects_oversized_input() -> Result<(), String> {
    let path = std::env::temp_dir().join(format!(
        "sts2-harness-workflow-binding-{}-oversized",
        std::process::id()
    ));
    std::fs::write(
        &path,
        vec![b'x'; super::MAX_REPLAY_SOURCE_BYTES as usize + 1],
    )
    .map_err(|error| format!("cannot write fixture: {error}"))?;
    let result = super::read_replay_source(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err());
    Ok(())
}

#[test]
fn durable_resume_rejects_a_cross_mode_fingerprint_change() -> Result<(), String> {
    let full_binding = WorkflowBinding::for_launch(false, false, None)?;
    let combat_binding = WorkflowBinding::for_launch(true, false, None)?;
    let full_fingerprint = fingerprint(&full_binding)?;
    let combat_fingerprint = fingerprint(&combat_binding)?;
    assert_ne!(
        full_fingerprint.config_digest,
        combat_fingerprint.config_digest
    );
    let lineage = lineage()?;
    let mut store = ExecutionStore::open_in_memory().map_err(|error| error.to_string())?;
    store
        .start_episode(&lineage, &full_fingerprint)
        .map_err(|error| error.to_string())?;
    assert!(matches!(
        store
            .resume_episode(&lineage.episode_id, &combat_fingerprint)
            .map_err(|error| error.to_string())?,
        ResumeState::ReconstructionRequired { .. }
    ));
    assert!(
        DurableHandle::from_store_for_test_with_binding(
            store,
            lineage,
            combat_fingerprint,
            combat_binding,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn combat_handle_allows_reward_completion_but_generic_completion_does_not() -> Result<(), String> {
    let binding = WorkflowBinding::for_launch(true, false, None)?;
    let fingerprint = fingerprint(&binding)?;
    let lineage = lineage()?;
    let mut store = ExecutionStore::open_in_memory().map_err(|error| error.to_string())?;
    store
        .start_episode(&lineage, &fingerprint)
        .map_err(|error| error.to_string())?;
    let handle =
        DurableHandle::from_store_for_test_with_binding(store, lineage, fingerprint, binding)?;
    let reward = reward_observation()?;
    handle.checkpoint(&reward, &json!([]))?;
    assert!(handle.complete_observation(&reward).is_err());
    handle.complete_combat_observation(&reward)?;
    handle.complete_combat_observation(&reward)?;
    Ok(())
}

#[test]
fn full_episode_handle_cannot_use_combat_reward_completion() -> Result<(), String> {
    let binding = WorkflowBinding::for_launch(false, false, None)?;
    let fingerprint = fingerprint(&binding)?;
    let lineage = lineage()?;
    let mut store = ExecutionStore::open_in_memory().map_err(|error| error.to_string())?;
    store
        .start_episode(&lineage, &fingerprint)
        .map_err(|error| error.to_string())?;
    let handle =
        DurableHandle::from_store_for_test_with_binding(store, lineage, fingerprint, binding)?;
    let reward = reward_observation()?;
    handle.checkpoint(&reward, &json!([]))?;
    assert!(handle.complete_combat_observation(&reward).is_err());
    Ok(())
}
