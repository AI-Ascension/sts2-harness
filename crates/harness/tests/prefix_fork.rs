// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used, dead_code)]

// AC1: a settled prefix at decision N replays exactly N settled actions with zero provider calls and
// admits a fork that dispatches a different next action once.
// AC2: siblings from one prefix share the source prefix and keep distinct trajectory/operation/
// context identities; duplicate identities and foreign prefixes are refused.
// AC3: a changed seed/setup/profile, a missing receipt, an unresolved action, a terminal source, an
// ambiguous legal binding and an observed divergence all stop before the mutation they guard.
// AC4: retained, destroyed, lost and changed targets, and a lost handoff reply, reconcile without a
// duplicate child or a skipped prefix verification.
// AC6: bounded declarations and the focused refusal matrix.

use sts2_harness::benchmark_manifest::prefix_fork::{
    ForkBinding, ForkBindingMismatch, HandoffError, HandoffMode, HandoffReconciliation,
    HandoffStage, LegalBinding, MAX_FORK_ORDINAL, MAX_PREFIX_RECEIPTS, MAX_SIBLINGS,
    PrefixBoundary, PrefixBoundaryError, PrefixForkRefusal, PrefixForkRequest, ReplayObservation,
    SiblingError, SiblingFork, SiblingSet, admit_handoff, admit_prefix_fork, next_handoff_stage,
    reconcile_lost_handoff,
};

fn binding(seed: &str) -> ForkBinding {
    ForkBinding {
        seed: seed.to_owned(),
        profile_id: String::from("profile-default"),
        build_digest: String::from("build-a"),
        compatibility_digest: String::from("compat-1"),
        prefix_digest: String::from("prefix-shared"),
    }
}

fn boundary(ordinal: u32) -> PrefixBoundary {
    let receipts: Vec<String> = (0..ordinal)
        .map(|index| format!("receipt-{index}"))
        .collect();
    PrefixBoundary {
        occurrence: String::from("occ-42"),
        ordinal,
        state_digest: String::from("state-42"),
        settled: true,
        terminal: false,
        receipts,
        expected_receipts: ordinal,
        legal_binding: LegalBinding::Resolved {
            action_key: String::from("play_card:strike"),
        },
    }
}

fn verified(settled_actions: u32) -> ReplayObservation {
    ReplayObservation::Verified {
        settled_actions,
        provider_calls: 0,
    }
}

fn request(
    requested: ForkBinding,
    recorded: ForkBinding,
    boundary: PrefixBoundary,
    replay: ReplayObservation,
) -> PrefixForkRequest {
    PrefixForkRequest {
        experiment_id: String::from("exp-1"),
        continuation_id: String::from("cont-1"),
        requested,
        recorded,
        boundary,
        replay,
    }
}

#[test]
fn a_verified_prefix_replays_exactly_and_admits_a_fork() {
    let shared = binding("seed-7");
    let plan = admit_prefix_fork(request(
        shared.clone(),
        shared.clone(),
        boundary(3),
        verified(3),
    ))
    .expect("a coherent fork is admitted");
    assert_eq!(plan.experiment_id, "exp-1");
    assert_eq!(plan.continuation_id, "cont-1");
    assert_eq!(plan.binding, shared);
    assert_eq!(plan.boundary_ordinal, 3);
    assert_eq!(plan.state_digest, "state-42");
    assert_eq!(plan.action_key, "play_card:strike");
}

#[test]
fn changed_bindings_are_refused_in_a_stable_category_order() {
    let recorded = binding("seed-7");
    let mut requested = recorded.clone();
    requested.seed = String::from("seed-8");
    requested.build_digest = String::from("build-b");
    let refusal = admit_prefix_fork(request(
        requested.clone(),
        recorded.clone(),
        boundary(3),
        verified(3),
    ))
    .expect_err("a changed binding is refused");
    assert_eq!(
        refusal,
        PrefixForkRefusal::BindingMismatch {
            reasons: vec![ForkBindingMismatch::Seed, ForkBindingMismatch::Build],
        }
    );
    assert!(!requested.is_compatible(&recorded));
    assert!(recorded.is_compatible(&recorded.clone()));
}

#[test]
fn a_prefix_replay_that_invokes_a_provider_is_refused() {
    let shared = binding("seed-7");
    let refusal = admit_prefix_fork(request(
        shared.clone(),
        shared,
        boundary(3),
        ReplayObservation::Verified {
            settled_actions: 3,
            provider_calls: 1,
        },
    ))
    .expect_err("a provider call during a prefix replay is refused");
    assert_eq!(
        refusal,
        PrefixForkRefusal::ProviderCallsDuringReplay { calls: 1 }
    );
}

#[test]
fn a_terminal_or_unsettled_source_is_refused() {
    let shared = binding("seed-7");
    let mut terminal = boundary(3);
    terminal.terminal = true;
    terminal.settled = false;
    let refusal = admit_prefix_fork(request(
        shared.clone(),
        shared.clone(),
        terminal,
        verified(3),
    ))
    .expect_err("a terminal source is refused");
    assert_eq!(refusal, PrefixForkRefusal::TerminalSource);
    let mut unsettled = boundary(3);
    unsettled.settled = false;
    let refusal = admit_prefix_fork(request(shared.clone(), shared, unsettled, verified(3)))
        .expect_err("an unresolved action is refused");
    assert_eq!(refusal, PrefixForkRefusal::UnresolvedAction);
}

#[test]
fn a_missing_settled_receipt_is_refused() {
    let shared = binding("seed-7");
    let mut selected = boundary(3);
    selected.receipts.pop();
    let refusal = admit_prefix_fork(request(shared.clone(), shared, selected, verified(3)))
        .expect_err("a missing receipt is refused");
    assert_eq!(
        refusal,
        PrefixForkRefusal::MissingReceipt {
            expected: 3,
            present: 2,
        }
    );
}

#[test]
fn an_ambiguous_legal_binding_is_refused() {
    let shared = binding("seed-7");
    let mut selected = boundary(3);
    selected.legal_binding = LegalBinding::Ambiguous;
    let refusal = admit_prefix_fork(request(shared.clone(), shared, selected, verified(3)))
        .expect_err("an ambiguous legal binding is refused");
    assert_eq!(refusal, PrefixForkRefusal::AmbiguousLegalBinding);
}

#[test]
fn an_observed_divergence_or_incomplete_replay_is_refused() {
    let shared = binding("seed-7");
    let diverged = admit_prefix_fork(request(
        shared.clone(),
        shared.clone(),
        boundary(3),
        ReplayObservation::Diverged { ordinal: 2 },
    ))
    .expect_err("an observed divergence is refused");
    assert_eq!(
        diverged,
        PrefixForkRefusal::ObservedDivergence { ordinal: 2 }
    );
    let incomplete = admit_prefix_fork(request(
        shared.clone(),
        shared.clone(),
        boundary(3),
        ReplayObservation::Incomplete {
            replayed: 1,
            expected: 3,
        },
    ))
    .expect_err("an incomplete replay is refused");
    assert_eq!(
        incomplete,
        PrefixForkRefusal::PrefixIncomplete {
            replayed: 1,
            expected: 3,
        }
    );
    let short = admit_prefix_fork(request(shared.clone(), shared, boundary(3), verified(2)))
        .expect_err("a verified replay short of the boundary is refused");
    assert_eq!(
        short,
        PrefixForkRefusal::PrefixIncomplete {
            replayed: 2,
            expected: 3,
        }
    );
}

#[test]
fn invalid_labels_and_out_of_range_declarations_are_refused() {
    let shared = binding("seed-7");
    let mut empty_experiment = request(shared.clone(), shared.clone(), boundary(3), verified(3));
    empty_experiment.experiment_id = String::new();
    assert_eq!(
        admit_prefix_fork(empty_experiment).expect_err("empty label is refused"),
        PrefixForkRefusal::InvalidLabel
    );
    let mut over_bound = boundary(3);
    over_bound.ordinal = MAX_FORK_ORDINAL + 1;
    assert_eq!(
        admit_prefix_fork(request(
            shared.clone(),
            shared.clone(),
            over_bound,
            verified(3)
        ))
        .expect_err("an out-of-range ordinal is refused"),
        PrefixForkRefusal::Boundary(PrefixBoundaryError::OrdinalOutOfRange)
    );
    let mut too_many = boundary(0);
    too_many.receipts = vec![String::from("r"); MAX_PREFIX_RECEIPTS + 1];
    too_many.expected_receipts = (MAX_PREFIX_RECEIPTS + 1) as u32;
    assert_eq!(
        admit_prefix_fork(request(
            shared.clone(),
            shared.clone(),
            too_many,
            verified(0)
        ))
        .expect_err("too many receipts are refused"),
        PrefixForkRefusal::Boundary(PrefixBoundaryError::TooManyReceipts)
    );
    let mut unbound = boundary(3);
    unbound.legal_binding = LegalBinding::Resolved {
        action_key: String::new(),
    };
    assert_eq!(
        admit_prefix_fork(request(shared.clone(), shared, unbound, verified(3)))
            .expect_err("an empty bound action is refused"),
        PrefixForkRefusal::Boundary(PrefixBoundaryError::InvalidBinding)
    );
}

fn sibling(index: usize) -> SiblingFork {
    SiblingFork {
        trajectory_id: format!("trajectory-{index}"),
        operation_id: format!("operation-{index}"),
        context_id: format!("context-{index}"),
        next_action: format!("play_card:action-{index}"),
    }
}

#[test]
fn siblings_share_one_prefix_and_keep_distinct_identities() {
    let mut set = SiblingSet::new("prefix-shared").expect("a bounded prefix is accepted");
    set.add("prefix-shared", sibling(0)).expect("first sibling");
    set.add("prefix-shared", sibling(1))
        .expect("second sibling");
    assert_eq!(set.siblings().len(), 2);
    assert_eq!(set.source_prefix_digest(), "prefix-shared");
    assert_ne!(
        set.siblings()[0].trajectory_id,
        set.siblings()[1].trajectory_id
    );
    assert_ne!(
        set.siblings()[0].operation_id,
        set.siblings()[1].operation_id
    );
    assert_ne!(set.siblings()[0].context_id, set.siblings()[1].context_id);
    let mut duplicate = sibling(1);
    duplicate.operation_id = String::from("operation-0");
    assert_eq!(
        set.add("prefix-shared", duplicate)
            .expect_err("a duplicate identity is refused"),
        SiblingError::DuplicateIdentity
    );
    assert_eq!(
        set.add("other-prefix", sibling(2))
            .expect_err("a foreign prefix is refused"),
        SiblingError::ForeignPrefix
    );
}

#[test]
fn the_sibling_bound_is_enforced() {
    let mut set = SiblingSet::new("prefix-shared").expect("a bounded prefix is accepted");
    for index in 0..MAX_SIBLINGS {
        set.add("prefix-shared", sibling(index))
            .expect("every sibling inside the bound is accepted");
    }
    assert_eq!(
        set.add("prefix-shared", sibling(MAX_SIBLINGS))
            .expect_err("the sibling bound is enforced"),
        SiblingError::Capacity
    );
    assert_eq!(set.siblings().len(), MAX_SIBLINGS);
}

#[test]
fn retained_destroyed_lost_and_changed_targets_are_adjudicated() {
    assert_eq!(
        admit_handoff(HandoffMode::Retained, true)
            .expect("a revalidated retained target is admitted"),
        HandoffMode::Retained
    );
    assert_eq!(
        admit_handoff(HandoffMode::Retained, false)
            .expect_err("a stale retained target is refused"),
        HandoffError::StaleRetainedTarget
    );
    for mode in [
        HandoffMode::Destroyed,
        HandoffMode::Lost,
        HandoffMode::Changed,
    ] {
        assert_eq!(
            admit_handoff(mode, true).expect_err("an unavailable target is refused"),
            HandoffError::UnavailableTarget(mode)
        );
    }
}

#[test]
fn a_lost_handoff_reply_never_duplicates_a_child_or_skips_verification() {
    assert_eq!(reconcile_lost_handoff(true), HandoffReconciliation::Adopted);
    assert_eq!(
        reconcile_lost_handoff(false),
        HandoffReconciliation::ReplayRequired
    );
}

#[test]
fn handoff_stages_only_advance_forward() {
    let chain = [
        (HandoffStage::PrefixSelected, HandoffStage::PrefixReplayed),
        (HandoffStage::PrefixReplayed, HandoffStage::HandoffIntended),
        (HandoffStage::HandoffIntended, HandoffStage::ChildAdmitted),
        (HandoffStage::ChildAdmitted, HandoffStage::ChildRunning),
    ];
    for (from, to) in chain {
        assert_eq!(
            next_handoff_stage(from, to).expect("the next stage is legal"),
            to
        );
    }
    let skipped = next_handoff_stage(HandoffStage::PrefixSelected, HandoffStage::HandoffIntended)
        .expect_err("a skipped stage is refused");
    assert_eq!(
        skipped,
        HandoffError::IllegalTransition {
            from: "prefix_selected",
            to: "handoff_intended",
        }
    );
    let backwards = next_handoff_stage(HandoffStage::ChildAdmitted, HandoffStage::PrefixReplayed)
        .expect_err("a backwards stage is refused");
    assert!(matches!(backwards, HandoffError::IllegalTransition { .. }));
    let terminal = next_handoff_stage(HandoffStage::ChildRunning, HandoffStage::ChildRunning)
        .expect_err("a terminal stage advances nowhere");
    assert!(matches!(terminal, HandoffError::IllegalTransition { .. }));
}
