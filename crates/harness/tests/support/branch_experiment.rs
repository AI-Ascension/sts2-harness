// SPDX-License-Identifier: MIT

//! Shared fixtures for the branch-experiment integration tests.
//!
//! Included via `#[path]` from the sibling `tests/branch_experiment*.rs` binaries so both files
//! build the same declarations, checkpoints and traces without duplicating the builders.

#![allow(dead_code)]

use sts2_harness::benchmark_manifest::branch_experiment::{
    BRANCH_EXPERIMENT_VERSION, BranchBudgets, BranchExperimentManifest, BranchOutcome, ChildPolicy,
    ForkStrategy, StopCondition, plan,
};
use sts2_harness::{
    ExactAssurance, ExactCheckpointId, ExactCheckpointReference, ExactStateDigest, ProjectionKey,
    TransitionRecord, TransitionTrace,
};

pub(crate) fn state(value: u64) -> ExactStateDigest {
    ExactStateDigest::parse(&format!("asc-state:v1:sha256:{value:064x}")).expect("state digest")
}

pub(crate) fn checkpoint(assurance: ExactAssurance) -> ExactCheckpointReference {
    ExactCheckpointReference {
        exact_state_digest: state(0),
        exact_checkpoint_id: ExactCheckpointId::parse(&format!(
            "asc-checkpoint:v1:sha256:{}",
            "c".repeat(64)
        ))
        .expect("checkpoint id"),
        boundary_kind: String::from("decision"),
        boundary_phase: String::from("combat"),
        assurance,
    }
}

pub(crate) fn trace(
    profile: &str,
    source: u64,
    after_base: u64,
    actions: &[&str],
) -> TransitionTrace {
    let source_state = state(source);
    let mut records = Vec::new();
    let mut before = source_state.clone();
    let mut previous: Option<String> = None;
    for (index, action) in actions.iter().enumerate() {
        let after = state(after_base + index as u64);
        let record = TransitionRecord {
            ordinal: index as u64,
            boundary_kind: String::from("decision"),
            boundary_phase: String::from("combat"),
            before: before.clone(),
            after: after.clone(),
            action_key: (*action).to_owned(),
            action_schema: String::from("action.v1"),
            catalog_witness: None,
            external_input_digest: None,
            previous_commitment: previous.clone(),
        };
        previous = Some(record.commitment().expect("commitment"));
        before = after;
        records.push(record);
    }
    TransitionTrace {
        profile: profile.to_owned(),
        source_state,
        records,
    }
}

pub(crate) fn settings(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

pub(crate) fn child(label: &str, seed: char, first_action: Option<&str>) -> ChildPolicy {
    ChildPolicy {
        child_label: label.to_owned(),
        settings_digest: settings(seed),
        first_action: first_action.map(str::to_owned),
    }
}

pub(crate) fn manifest(
    strategy: ForkStrategy,
    children: Vec<ChildPolicy>,
) -> BranchExperimentManifest {
    BranchExperimentManifest {
        version: BRANCH_EXPERIMENT_VERSION.to_owned(),
        experiment_id: String::from("exp-1"),
        benchmark_ref: String::from("bench-1"),
        fork_point: checkpoint(ExactAssurance::RestoreVerified),
        strategy,
        context_policy: String::from("context-default"),
        stop_conditions: vec![
            StopCondition::TerminalOutcome,
            StopCondition::BudgetExhausted,
        ],
        budgets: BranchBudgets {
            max_decisions_per_child: 10,
            max_total_decisions: 20,
            max_total_duration_millis: 60_000,
            max_provider_spend_micros: Some(1_000_000),
            max_concurrency: 1,
        },
        children,
    }
}

pub(crate) fn alternate(first: &str, second: &str) -> BranchExperimentManifest {
    manifest(
        ForkStrategy::AlternativeFirstAction,
        vec![child("a", 'a', Some(first)), child("b", 'b', Some(second))],
    )
}

pub(crate) fn key(manifest: &BranchExperimentManifest, label: &str) -> String {
    plan(manifest)
        .expect("plan")
        .into_iter()
        .find(|trial| trial.child_label == label)
        .expect("planned trial")
        .trial_key
}

pub(crate) fn completed(key: &str) -> BranchOutcome {
    BranchOutcome::completed(key, trace("action.v1", 0, 100, &["x"]))
        .with_start(ExactAssurance::RestoreVerified)
}

pub(crate) fn projection_key(seed: u8) -> ProjectionKey {
    ProjectionKey::new(&[seed; 32]).expect("projection key")
}
