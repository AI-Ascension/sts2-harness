// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    AncestrySplit, BranchPolicy, ExactAssurance, ExactCheckpointId, ExactCheckpointReference,
    ExactStateDigest, Experiment, ExperimentError, LineageError, OccurrenceGraph, OccurrenceId,
    OccurrenceRecord, split_by_ancestry,
};

fn digest(seed: char) -> ExactStateDigest {
    ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("digest is valid")
}

fn checkpoint(seed: char) -> ExactCheckpointId {
    ExactCheckpointId::parse(&format!(
        "asc-checkpoint:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("checkpoint is valid")
}

fn start() -> ExactCheckpointReference {
    ExactCheckpointReference {
        exact_state_digest: digest('a'),
        exact_checkpoint_id: checkpoint('b'),
        boundary_kind: "decision".to_owned(),
        boundary_phase: "COMBAT".to_owned(),
        assurance: ExactAssurance::RestoreVerified,
    }
}

fn policy(label: &str, seed: char) -> BranchPolicy {
    BranchPolicy {
        label: label.to_owned(),
        settings_digest: format!("sha256:{}", seed.to_string().repeat(64)),
    }
}

fn id(value: &str) -> OccurrenceId {
    OccurrenceId::parse(value).expect("identifier is valid")
}

fn record(
    occurrence: &str,
    parent: Option<&str>,
    action: Option<&str>,
    state: char,
) -> OccurrenceRecord {
    OccurrenceRecord {
        occurrence_id: id(occurrence),
        parent: parent.map(id),
        parent_checkpoint: None,
        action_key: action.map(str::to_owned),
        state_digest: digest(state),
        experiment_id: None,
    }
}

#[test]
fn branches_share_one_start_and_keep_policy_outside_identity() {
    let start = start();
    let mut experiment = Experiment::new("trial:one", start.clone()).expect("experiment");
    experiment
        .fork(
            "branch:alpha",
            id("run:alpha"),
            policy("model-alpha", 'c'),
            "exp:one/alpha",
        )
        .expect("first branch");
    experiment
        .fork(
            "branch:beta",
            id("run:beta"),
            policy("model-beta", 'd'),
            "exp:one/beta",
        )
        .expect("second branch");
    assert_eq!(experiment.branches().len(), 2);
    assert!(experiment.has_branches());
    assert_eq!(experiment.start, start);
    let scopes: std::collections::BTreeSet<&str> = experiment
        .branches()
        .iter()
        .map(|branch| branch.write_scope.as_str())
        .collect();
    assert_eq!(scopes.len(), 2);
    assert_ne!(
        experiment.branches()[0].policy,
        experiment.branches()[1].policy
    );
    assert_eq!(experiment.start.exact_state_digest, digest('a'));
}

#[test]
fn opaque_handles_must_not_expose_identity() {
    let mut experiment = Experiment::new("trial:two", start()).expect("experiment");
    let leaky = "a".repeat(64);
    assert_eq!(
        experiment
            .fork(&leaky, id("run:leak"), policy("model", 'c'), "exp:two/leak")
            .expect_err("identity leak rejected"),
        ExperimentError::InvalidHandle
    );
    assert!(
        experiment
            .fork(
                "branch:ok",
                id("run:ok"),
                policy("model", 'c'),
                "exp:two/ok"
            )
            .is_ok()
    );
}

#[test]
fn duplicate_handles_scopes_and_labels_are_rejected() {
    let mut experiment = Experiment::new("trial:three", start()).expect("experiment");
    experiment
        .fork(
            "branch:one",
            id("run:one"),
            policy("model", 'c'),
            "exp:three/one",
        )
        .expect("branch");
    assert_eq!(
        experiment
            .fork(
                "branch:one",
                id("run:two"),
                policy("model", 'd'),
                "exp:three/two"
            )
            .expect_err("duplicate handle"),
        ExperimentError::InvalidHandle
    );
    assert_eq!(
        experiment
            .fork(
                "branch:two",
                id("run:two"),
                policy("model", 'd'),
                "exp:three/one"
            )
            .expect_err("duplicate scope"),
        ExperimentError::DuplicateScope
    );
    assert_eq!(
        experiment
            .fork("", id("run:three"), policy("model", 'd'), "exp:three/three")
            .expect_err("empty handle"),
        ExperimentError::InvalidLabel
    );
    let bad_settings = BranchPolicy {
        label: "model".to_owned(),
        settings_digest: "not-a-digest".to_owned(),
    };
    assert_eq!(
        experiment
            .fork(
                "branch:three",
                id("run:three"),
                bad_settings,
                "exp:three/three"
            )
            .expect_err("bad settings digest"),
        ExperimentError::InvalidLabel
    );
}

#[test]
fn ancestry_splits_merge_duplicate_starts_and_never_overlap() {
    let mut graph = OccurrenceGraph::new();
    for record in [
        record("f1:root", None, None, 'a'),
        record("f1:one", Some("f1:root"), Some("play_card"), 'b'),
        record("f2:root", None, None, 'a'),
        record("f2:one", Some("f2:root"), Some("end_turn"), 'c'),
        record("f3:root", None, None, 'd'),
        record("f3:one", Some("f3:root"), Some("play_card"), 'e'),
    ] {
        graph.insert(record).expect("record inserts");
    }
    let split = split_by_ancestry(&graph, 50, 50).expect("split builds");
    assert!(split.overlap().is_empty());
    assert_eq!(split.len(), graph.len());
    let (train, validation, test) = split.sizes();
    assert_eq!(test, 0);
    assert!(train > 0 && validation > 0);
    let split_of = |target: &str| -> &'static str {
        let target = id(target);
        if split.train.contains(&target) {
            "train"
        } else if split.validation.contains(&target) {
            "validation"
        } else {
            "test"
        }
    };
    assert_eq!(split_of("f1:root"), split_of("f2:root"));
    assert_eq!(split_of("f1:one"), split_of("f2:one"));
    assert_ne!(split_of("f1:root"), split_of("f3:root"));
}

#[test]
fn invalid_split_percentages_are_rejected() {
    let mut graph = OccurrenceGraph::new();
    graph
        .insert(record("only:root", None, None, 'a'))
        .expect("record inserts");
    assert_eq!(
        split_by_ancestry(&graph, 60, 50).expect_err("percentages exceed 100"),
        LineageError::InvalidRecord
    );
    let split: AncestrySplit = split_by_ancestry(&graph, 0, 0).expect("all to test");
    assert_eq!(split.sizes(), (0, 0, 1));
    assert_eq!(split.overlap(), Vec::new());
}
