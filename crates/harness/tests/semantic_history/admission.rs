// SPDX-License-Identifier: MIT

//! Admission: the vocabulary's refusal contract, clause by clause.

use super::*;

#[test]
fn a_captured_event_is_admitted_with_its_details() {
    let admitted = sts2_harness::semantic_history::admit_batch(
        &binding(),
        &batch(
            "b",
            1,
            vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
        ),
    )
    .expect("batch is admitted");
    assert_eq!(admitted.len(), 2);
    assert!(admitted.iter().all(|record| record.is_observed()));
    assert_eq!(
        admitted[1]
            .event
            .causal_parent
            .as_ref()
            .unwrap()
            .stated_parent(),
        Some("e1")
    );
}

#[test]
fn a_kind_that_requires_a_target_is_refused_without_one() {
    let mut event = damage("e1", 1, None);
    event.subjects = vec![actor("player-1")];
    assert_eq!(
        error(batch("b", 1, vec![event])),
        SemanticHistoryRefusal::MissingTarget
    );
}

#[test]
fn a_kind_that_requires_an_actor_is_refused_without_one() {
    let mut event = card_played("e1", 1);
    event.subjects = Vec::new();
    assert_eq!(
        error(batch("b", 1, vec![event])),
        SemanticHistoryRefusal::SubjectRole
    );
}

#[test]
fn a_subject_minted_in_a_definition_namespace_is_refused() {
    let mut event = card_played("e1", 1);
    event.subjects = vec![SemanticEventSubject {
        role: SemanticSubjectRole::Actor,
        namespace: SemanticIdentityNamespace::Definition,
        subject_id: "card.strike".to_owned(),
    }];
    assert_eq!(
        error(batch("b", 1, vec![event])),
        SemanticHistoryRefusal::SubjectNamespace
    );
}

#[test]
fn a_quantity_on_a_kind_that_carries_none_is_refused() {
    let mut event = card_played("e1", 1);
    event.value = Some(SemanticQuantity {
        amount: 0,
        unit: "health".to_owned(),
    });
    assert_eq!(
        error(batch("b", 1, vec![event])),
        SemanticHistoryRefusal::UnexpectedDetail
    );
}

#[test]
fn a_kind_that_requires_a_quantity_is_refused_without_one() {
    let mut event = damage("e1", 1, None);
    event.value = None;
    assert_eq!(
        error(batch("b", 1, vec![event])),
        SemanticHistoryRefusal::MissingDetail
    );
}

#[test]
fn a_reference_on_a_kind_that_names_none_is_refused() {
    let mut event = damage("e1", 1, None);
    event.reference = Some(SemanticReference {
        entity_kind: "card".to_owned(),
        namespaced_id: "card.strike".to_owned(),
    });
    assert_eq!(
        error(batch("b", 1, vec![event])),
        SemanticHistoryRefusal::UnexpectedDetail
    );
}

#[test]
fn a_causal_parent_on_a_kind_that_admits_none_is_refused() {
    let mut event = card_played("e1", 1);
    event.causal_parent = Some(SemanticCausalParent {
        parent_event_id: Some("e0".to_owned()),
        provenance: SemanticCausalProvenance::Stated,
    });
    assert_eq!(
        error(batch("b", 1, vec![event])),
        SemanticHistoryRefusal::CausalityNotAdmitted
    );
}

#[test]
fn a_parent_identity_and_provenance_that_disagree_are_refused() {
    let mut event = damage("e2", 2, None);
    event.causal_parent = Some(SemanticCausalParent {
        parent_event_id: Some("e1".to_owned()),
        provenance: SemanticCausalProvenance::NotStated,
    });
    assert_eq!(
        error(batch("b", 1, vec![card_played("e1", 1), event])),
        SemanticHistoryRefusal::CausalShape
    );
}

#[test]
fn an_imported_event_may_not_state_a_parent() {
    let mut event = damage("e2", 2, Some("e1"));
    event.origin = Some(SemanticEventOrigin::Imported);
    assert_eq!(
        error(batch("b", 1, vec![card_played("e1", 1), event])),
        SemanticHistoryRefusal::ImportedCausality
    );
}

#[test]
fn a_stated_parent_absent_from_the_history_is_refused() {
    assert_eq!(
        error(batch(
            "b",
            1,
            vec![card_played("e1", 1), damage("e2", 2, Some("missing"))]
        )),
        SemanticHistoryRefusal::StatedParentUnknown
    );
}

#[test]
fn a_stated_parent_that_does_not_precede_its_child_is_refused() {
    assert_eq!(
        error(batch(
            "b",
            1,
            vec![damage("e1", 1, Some("e2")), card_played("e2", 2)]
        )),
        SemanticHistoryRefusal::StatedParentNotBefore
    );
}

#[test]
fn a_gap_keeps_its_sequence_number_and_carries_no_gameplay_detail() {
    let mut b = batch(
        "b",
        1,
        vec![
            card_played("e1", 1),
            gap("g2", 2, SemanticCoverageStatus::Dropped),
            card_played("e3", 3),
        ],
    );
    b.window.intervals = vec![dropped(2, 2)];
    let admitted = sts2_harness::semantic_history::admit_batch(&binding(), &b)
        .expect("a disclosed gap is admitted");
    assert_eq!(admitted.len(), 3);
    assert!(!admitted[1].is_observed());
    assert_eq!(admitted[1].event.sequence, 2);
    assert_eq!(admitted[2].event.sequence, 3);
}

#[test]
fn a_gap_that_supplies_a_kind_is_refused() {
    let mut event = gap("g2", 2, SemanticCoverageStatus::Dropped);
    event.kind = Some(SemanticEventKind::Damage);
    let mut b = batch("b", 1, vec![card_played("e1", 1), event]);
    b.window.intervals = vec![SemanticCoverageInterval {
        status: SemanticCoverageStatus::Dropped,
        first_sequence: 2,
        last_sequence: 2,
    }];
    assert_eq!(error(b), SemanticHistoryRefusal::CoverageShape);
}

#[test]
fn a_gap_outside_any_declared_interval_is_refused() {
    assert_eq!(
        error(batch(
            "b",
            1,
            vec![
                card_played("e1", 1),
                gap("g2", 2, SemanticCoverageStatus::Dropped)
            ]
        )),
        SemanticHistoryRefusal::GapOutsideCapture
    );
}

#[test]
fn a_captured_event_inside_a_declared_gap_is_refused() {
    let mut b = batch("b", 1, vec![card_played("e1", 1), card_played("e2", 2)]);
    b.window.intervals = vec![SemanticCoverageInterval {
        status: SemanticCoverageStatus::Dropped,
        first_sequence: 2,
        last_sequence: 2,
    }];
    assert_eq!(error(b), SemanticHistoryRefusal::CapturedInsideGap);
}

#[test]
fn a_non_contiguous_sequence_is_refused() {
    assert_eq!(
        error(batch(
            "b",
            1,
            vec![card_played("e1", 1), card_played("e2", 3)]
        )),
        SemanticHistoryRefusal::NonMonotonicSequence
    );
}

#[test]
fn two_records_sharing_an_event_identity_are_refused() {
    assert_eq!(
        error(batch(
            "b",
            1,
            vec![card_played("e1", 1), card_played("e1", 2)]
        )),
        SemanticHistoryRefusal::DuplicateEvent
    );
}

#[test]
fn a_window_that_contradicts_where_capture_began_is_refused() {
    let mut b = batch("b", 1, vec![card_played("e1", 1)]);
    b.window.history_before_capture = true;
    assert_eq!(error(b), SemanticHistoryRefusal::WindowContradiction);
}

#[test]
fn an_identity_carrying_a_traversal_segment_is_refused() {
    assert_eq!(
        error(batch("b", 1, vec![card_played("..", 1)])),
        SemanticHistoryRefusal::Identity
    );
    assert_eq!(
        error(batch("b", 1, vec![card_played("a/b", 1)])),
        SemanticHistoryRefusal::Identity
    );
}
