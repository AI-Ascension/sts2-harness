// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryAppend, SemanticHistoryCausalParent, SemanticHistoryError,
    SemanticHistoryEventInput, SemanticHistoryKind, SemanticHistoryQuery, SemanticHistoryReader,
    SemanticHistoryStore, SemanticHistorySubjectRole, SemanticHistoryValue,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

fn append(
    store: &mut SemanticHistoryStore,
    input: SemanticHistoryEventInput,
    parent: SemanticHistoryCausalParent,
) -> Result<SemanticHistoryAppend, SemanticHistoryError> {
    store.append(fixture::ROOT, input, parent)
}

/// Appends one event that the kind rules must refuse, and returns the reason.
fn refused(
    store: &mut SemanticHistoryStore,
    input: SemanticHistoryEventInput,
    parent: SemanticHistoryCausalParent,
) -> SemanticHistoryError {
    append(store, input, parent).expect_err("the event is refused")
}

#[test]
fn a_kind_that_requires_an_actor_refuses_an_event_that_names_none() {
    let mut store = fixture::store();
    let mut input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    input.subjects.clear();
    // An effect that cannot name who caused it is not an authoritative event.
    assert_eq!(
        refused(&mut store, input, SemanticHistoryCausalParent::NotStated),
        SemanticHistoryError::MissingSubject("actor")
    );
}

#[test]
fn a_target_is_refused_by_a_kind_that_does_not_act_on_one() {
    let mut store = fixture::store();
    let mut input = fixture::event("event_1", SemanticHistoryKind::ChoiceMade, 1, None);
    input.subjects.push(fixture::target("instance_enemy"));
    // A target where there is no targeted action would be read as "this choice acted on that".
    assert_eq!(
        refused(&mut store, input, SemanticHistoryCausalParent::NotStated),
        SemanticHistoryError::UnexpectedSubjectRole("target")
    );
}

#[test]
fn naming_one_end_of_an_event_twice_is_refused() {
    let mut store = fixture::store();
    let mut input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    input.subjects.push(fixture::actor("instance_rival"));
    // Two actors would make the event's cause ambiguous rather than ranged over.
    assert_eq!(
        refused(&mut store, input, SemanticHistoryCausalParent::NotStated),
        SemanticHistoryError::DuplicateSubjectRole("actor")
    );
}

#[test]
fn a_live_subject_may_not_alias_the_event_the_branch_or_the_run() {
    for alias in ["event_1", fixture::ROOT, "run_0001"] {
        let mut store = fixture::store();
        let mut input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
        input.subjects = vec![fixture::actor(alias)];
        assert_eq!(
            refused(&mut store, input, SemanticHistoryCausalParent::NotStated),
            SemanticHistoryError::IdentityNamespaceCollision("actor"),
            "{alias} is not a live instance identity"
        );
    }
}

#[test]
fn a_content_naming_kind_requires_its_reference_and_others_refuse_one() {
    let mut store = fixture::store();
    let mut missing = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    missing.reference = None;
    assert_eq!(
        refused(&mut store, missing, SemanticHistoryCausalParent::NotStated),
        SemanticHistoryError::InvalidField("reference")
    );
    let mut unexpected = fixture::event("event_2", SemanticHistoryKind::ChoiceMade, 1, None);
    unexpected.reference = Some(fixture::reference("card_strike"));
    assert_eq!(
        refused(
            &mut store,
            unexpected,
            SemanticHistoryCausalParent::NotStated
        ),
        SemanticHistoryError::InvalidField("reference")
    );
}

#[test]
fn a_quantity_reporting_kind_refuses_a_value_where_none_is_reported() {
    let mut store = fixture::store();
    let mut input = fixture::event("event_1", SemanticHistoryKind::ChoiceMade, 1, None);
    input.value = Some(fixture::quantity(1, "hp"));
    // A choice that reports no quantity must not carry one; the amount would be read as measured.
    assert_eq!(
        refused(&mut store, input, SemanticHistoryCausalParent::NotStated),
        SemanticHistoryError::InvalidField("value")
    );
}

#[test]
fn a_kind_that_cannot_be_caused_refuses_a_stated_parent() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::ChoiceMade,
        1,
    );
    let input = fixture::event("event_2", SemanticHistoryKind::RoomTransitioned, 2, None);
    // A room transition has no causal parent to state; recording one would invent causality.
    assert_eq!(
        refused(&mut store, input, fixture::stated_parent("event_1")),
        SemanticHistoryError::Causality
    );
}

#[test]
fn a_disclosed_gap_refuses_any_observed_detail() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 2),
    )
    .expect("store opens");
    let mut named = fixture::gap_event("event_2", 2);
    named.subjects = vec![fixture::actor("instance_hero")];
    // A gap that names a subject would close itself with detail the capture said it could not get.
    assert_eq!(
        refused(&mut store, named, SemanticHistoryCausalParent::NotStated),
        SemanticHistoryError::InvalidField("subjects")
    );
    let mut valued = fixture::gap_event("event_3", 2);
    valued.value = Some(SemanticHistoryValue::Unavailable {
        reason: "not_observed".to_owned(),
    });
    assert_eq!(
        refused(&mut store, valued, SemanticHistoryCausalParent::NotStated),
        SemanticHistoryError::InvalidField("value")
    );
    assert_eq!(
        append(
            &mut store,
            fixture::gap_event("event_4", 2),
            SemanticHistoryCausalParent::NotStated
        )
        .expect("a bare gap is admitted"),
        SemanticHistoryAppend::Recorded
    );
}

#[test]
fn both_ends_of_a_targeted_event_are_stored_with_their_roles() {
    let mut store = fixture::store();
    fixture::append_quantity(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::Damage,
        1,
        7,
    );
    let stored = store.event(fixture::ROOT, "event_1").expect("event");
    assert_eq!(stored.input.subjects.len(), 2);
    let roles: Vec<SemanticHistorySubjectRole> = stored
        .input
        .subjects
        .iter()
        .map(|subject| subject.role)
        .collect();
    assert_eq!(
        roles,
        vec![
            SemanticHistorySubjectRole::Actor,
            SemanticHistorySubjectRole::Target
        ]
    );
    assert_eq!(stored.input.subjects[0].identity, "instance_hero");
    assert_eq!(stored.input.subjects[1].identity, "instance_enemy");
    assert_eq!(stored.input.episode, 1);
    assert!(stored.input.reference.is_none());
}

#[test]
fn a_subject_filter_matches_either_end_and_never_returns_an_event_twice() {
    let mut store = fixture::store();
    // One self-targeted hit names the same live instance as actor and as target.
    let mut input = fixture::event(
        "event_1",
        SemanticHistoryKind::Damage,
        1,
        Some(fixture::quantity(3, "hp")),
    );
    input.subjects = vec![
        fixture::actor("instance_hero"),
        fixture::target("instance_hero"),
    ];
    append(&mut store, input, SemanticHistoryCausalParent::NotStated).expect("append");
    let reader = SemanticHistoryReader::open(&store, fixture::ROOT).expect("reader");
    for subject_id in ["instance_hero", "instance_enemy"] {
        let mut query = SemanticHistoryQuery::branch(fixture::ROOT, 10);
        query.subject_id = Some(subject_id.to_owned());
        let page = reader.page(&query, None).expect("page");
        let expected = usize::from(subject_id == "instance_hero");
        assert_eq!(page.events.len(), expected, "{subject_id}");
    }
    // An episode axis selects by the number the event carries.
    let mut query = SemanticHistoryQuery::branch(fixture::ROOT, 10);
    query.episode = Some(2);
    assert!(reader.page(&query, None).expect("page").events.is_empty());
    query.episode = Some(1);
    assert_eq!(reader.page(&query, None).expect("page").events.len(), 1);
}
