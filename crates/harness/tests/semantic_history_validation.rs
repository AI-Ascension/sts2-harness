// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use serde_json::json;
use sts2_harness::semantic_history::{
    MAX_HISTORY_PAGE, SemanticHistoryCaptureWindow, SemanticHistoryCausalParent,
    SemanticHistoryCoverage, SemanticHistoryCoverageInterval, SemanticHistoryCoverageStatus,
    SemanticHistoryError, SemanticHistoryEventInput, SemanticHistoryKind, SemanticHistoryLineage,
    SemanticHistoryNamespace, SemanticHistoryQuery, SemanticHistorySubject,
    SemanticHistorySubjectRole, is_opaque_history_identity,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

fn interval(from: u64, to: u64) -> SemanticHistoryCoverageInterval {
    SemanticHistoryCoverageInterval {
        from_sequence: from,
        to_sequence: to,
        status: SemanticHistoryCoverageStatus::Dropped,
        label: "capture_dropped".to_owned(),
    }
}

#[test]
fn a_query_limit_outside_the_page_bound_is_refused() {
    assert_eq!(
        SemanticHistoryQuery::branch(fixture::ROOT, 0).validate(),
        Err(SemanticHistoryError::Bounds)
    );
    assert_eq!(
        SemanticHistoryQuery::branch(fixture::ROOT, MAX_HISTORY_PAGE + 1).validate(),
        Err(SemanticHistoryError::Bounds)
    );
    assert_eq!(
        SemanticHistoryQuery::branch(fixture::ROOT, MAX_HISTORY_PAGE).validate(),
        Ok(())
    );
    assert_eq!(
        SemanticHistoryQuery::branch(fixture::ROOT, 1).validate(),
        Ok(())
    );
}
#[test]
fn a_query_with_an_inverted_sequence_window_is_refused() {
    let mut query = SemanticHistoryQuery::branch(fixture::ROOT, 4);
    query.from_sequence = Some(3);
    query.to_sequence = Some(2);
    assert_eq!(query.validate(), Err(SemanticHistoryError::Bounds));
    query.to_sequence = Some(3);
    assert_eq!(query.validate(), Ok(()));
    query.from_sequence = None;
    assert_eq!(query.validate(), Ok(()));
}
#[test]
fn a_query_naming_a_non_opaque_subject_is_refused() {
    let mut query = SemanticHistoryQuery::branch(fixture::ROOT, 4);
    query.subject_id = Some("/etc/passwd".to_owned());
    assert_eq!(
        query.validate(),
        Err(SemanticHistoryError::NonOpaqueIdentity("query.subject_id"))
    );
    query.subject_id = None;
    // An episode is a bounded number, not an opaque identity, so any number is a valid axis.
    query.episode = Some(3);
    assert_eq!(query.validate(), Ok(()));
}
#[test]
fn a_query_digest_binds_every_axis() {
    let base = SemanticHistoryQuery::branch(fixture::ROOT, 8);
    let same = SemanticHistoryQuery::branch(fixture::ROOT, 8);
    let digest = base.digest().expect("digest");
    assert_eq!(digest, same.digest().expect("digest"));
    assert!(is_opaque_history_identity(&digest));

    let mut other = SemanticHistoryQuery::branch(fixture::ROOT, 9);
    assert_ne!(digest, other.digest().expect("digest"));
    other = base.clone();
    other.kind = Some(SemanticHistoryKind::Damage);
    assert_ne!(digest, other.digest().expect("digest"));
    other = base.clone();
    other.origin = Some(sts2_harness::semantic_history::SemanticHistoryOrigin::Imported);
    assert_ne!(digest, other.digest().expect("digest"));
    other = base.clone();
    other.from_sequence = Some(2);
    assert_ne!(digest, other.digest().expect("digest"));
}
#[test]
fn a_query_refuses_an_unknown_axis() {
    let value =
        serde_json::to_value(SemanticHistoryQuery::branch(fixture::ROOT, 4)).expect("encode");
    serde_json::from_value::<SemanticHistoryQuery>(value.clone()).expect("round trip");
    let mut object = value.as_object().expect("object").clone();
    object.insert("unexpected_axis".to_owned(), json!(1));
    // An axis this boundary does not know is refused rather than ignored.
    assert!(
        serde_json::from_value::<SemanticHistoryQuery>(serde_json::Value::Object(object)).is_err()
    );
}
#[test]
fn an_event_input_refuses_an_unknown_field() {
    let input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    let value = serde_json::to_value(&input).expect("encode");
    assert_eq!(
        serde_json::from_value::<SemanticHistoryEventInput>(value.clone()).expect("round trip"),
        input
    );
    let mut object = value.as_object().expect("object").clone();
    object.insert("extra_field".to_owned(), json!(true));
    assert!(
        serde_json::from_value::<SemanticHistoryEventInput>(serde_json::Value::Object(object))
            .is_err()
    );
}
#[test]
fn a_lineage_edge_that_forks_from_itself_is_refused() {
    let mut edge = SemanticHistoryLineage {
        branch_id: "branch_child".to_owned(),
        parent_branch_id: Some("branch_child".to_owned()),
        fork_sequence: 1,
        authority_epoch: 1,
    };
    assert_eq!(edge.validate(), Err(SemanticHistoryError::Lineage));
    edge.parent_branch_id = Some(fixture::ROOT.to_owned());
    assert_eq!(edge.validate(), Ok(()));
    edge.parent_branch_id = Some("/etc".to_owned());
    assert_eq!(
        edge.validate(),
        Err(SemanticHistoryError::NonOpaqueIdentity(
            "lineage.parent_branch_id"
        ))
    );
    edge.parent_branch_id = None;
    assert_eq!(edge.validate(), Ok(()));
}
#[test]
fn a_coverage_interval_that_is_observed_or_inverted_is_refused() {
    let valid = interval(2, 3);
    assert_eq!(valid.validate(), Ok(()));
    assert!(valid.contains(2) && valid.contains(3) && !valid.contains(4));
    let mut bad = valid.clone();
    bad.status = SemanticHistoryCoverageStatus::Captured;
    assert_eq!(bad.validate(), Err(SemanticHistoryError::Coverage));
    bad = valid.clone();
    bad.from_sequence = 4;
    assert_eq!(bad.validate(), Err(SemanticHistoryError::Coverage));
    bad = valid;
    bad.label = String::new();
    assert_eq!(
        bad.validate(),
        Err(SemanticHistoryError::InvalidLabel(
            "coverage_interval.label"
        ))
    );
    // A captured event may not carry a label; the label would stand in for an observed value.
    assert_eq!(SemanticHistoryCoverage::captured().validate(), Ok(()));
    assert_eq!(
        SemanticHistoryCoverage::gap(SemanticHistoryCoverageStatus::Dropped, "capture_dropped")
            .validate(),
        Ok(())
    );
    assert_eq!(
        SemanticHistoryCoverage {
            status: SemanticHistoryCoverageStatus::Captured,
            label: Some("looks_bad".to_owned())
        }
        .validate(),
        Err(SemanticHistoryError::Coverage)
    );
}
#[test]
fn a_window_with_an_overlapping_or_pre_capture_span_is_refused() {
    let mut window = SemanticHistoryCaptureWindow::complete(1);
    window.intervals.push(interval(1, 3));
    window.intervals.push(interval(3, 4));
    assert_eq!(window.validate(), Err(SemanticHistoryError::Coverage));
    window.intervals = vec![interval(0, 1)];
    assert_eq!(window.validate(), Err(SemanticHistoryError::Coverage));
    window.intervals = vec![interval(1, 2), interval(4, 5)];
    assert_eq!(window.validate(), Ok(()));
    assert_eq!(
        window.gap_at(1).map(|gap| gap.status),
        Some(SemanticHistoryCoverageStatus::Dropped)
    );
    assert!(window.gap_at(3).is_none());
    assert!(window.is_before_capture(0));
    assert!(!window.is_before_capture(1));
}
#[test]
fn a_subject_outside_the_live_instance_namespace_is_refused() {
    for namespace in [
        SemanticHistoryNamespace::Definition,
        SemanticHistoryNamespace::Action,
        SemanticHistoryNamespace::Event,
    ] {
        let subject = SemanticHistorySubject {
            role: SemanticHistorySubjectRole::Actor,
            namespace,
            identity: "thing_1".to_owned(),
        };
        assert_eq!(
            subject.validate("subject"),
            Err(SemanticHistoryError::WrongSubjectNamespace("actor")),
            "a {} subject is refused",
            namespace.name()
        );
    }
    assert_eq!(fixture::actor("instance_hero").validate("subject"), Ok(()));
    let bad = SemanticHistorySubject {
        role: SemanticHistorySubjectRole::Target,
        namespace: SemanticHistoryNamespace::LiveInstance,
        identity: "a/b".to_owned(),
    };
    assert_eq!(
        bad.validate("subject"),
        Err(SemanticHistoryError::NonOpaqueIdentity("subject"))
    );
}
#[test]
fn a_causal_parent_that_mixes_its_arms_is_refused() {
    // "We do not know why" can never be decoded as "this was the cause".
    assert!(
        serde_json::from_value::<SemanticHistoryCausalParent>(
            json!({"state": "not_stated", "event_id": "event_1"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<SemanticHistoryCausalParent>(json!({"state": "stated"})).is_err()
    );
    assert!(
        serde_json::from_value::<SemanticHistoryCausalParent>(json!({"state": "assumed"})).is_err()
    );
    assert_eq!(
        serde_json::from_value::<SemanticHistoryCausalParent>(json!({"state": "not_stated"}))
            .expect("unstated"),
        SemanticHistoryCausalParent::NotStated
    );
    assert_eq!(
        serde_json::from_value::<SemanticHistoryCausalParent>(
            json!({"state": "stated", "event_id": "event_1"})
        )
        .expect("stated"),
        SemanticHistoryCausalParent::Stated {
            event_id: "event_1".to_owned()
        }
    );
}
#[test]
fn a_stated_causal_parent_must_name_a_bounded_opaque_identity() {
    assert_eq!(SemanticHistoryCausalParent::NotStated.validate(), Ok(()));
    assert_eq!(
        SemanticHistoryCausalParent::NotStated.stated_event_id(),
        None
    );
    assert!(!SemanticHistoryCausalParent::NotStated.is_stated());
    let stated = SemanticHistoryCausalParent::Stated {
        event_id: "event_1".to_owned(),
    };
    assert_eq!(stated.stated_event_id(), Some("event_1"));
    assert!(stated.is_stated());
    assert_eq!(stated.validate(), Ok(()));
    assert_eq!(
        SemanticHistoryCausalParent::Stated {
            event_id: "/etc/passwd".to_owned()
        }
        .validate(),
        Err(SemanticHistoryError::NonOpaqueIdentity(
            "causal_parent.event_id"
        ))
    );
    assert_eq!(
        SemanticHistoryCausalParent::Stated {
            event_id: String::new()
        }
        .validate(),
        Err(SemanticHistoryError::InvalidIdentity(
            "causal_parent.event_id"
        ))
    );
}
#[test]
fn an_error_reports_a_stable_reason_without_producer_text() {
    assert_eq!(
        SemanticHistoryError::InvalidLabel("value.unit").to_string(),
        "semantic history: invalid label: value.unit"
    );
    assert_eq!(
        SemanticHistoryError::NonOpaqueIdentity("event_id").to_string(),
        "semantic history: non-opaque identity: event_id"
    );
    assert_eq!(
        SemanticHistoryError::Scope.to_string(),
        "semantic history: Scope"
    );
    assert_eq!(
        SemanticHistoryError::Persistence("disk".to_owned()).to_string(),
        "semantic history: Persistence(\"disk\")"
    );
    let error: &dyn std::error::Error = &SemanticHistoryError::Authority;
    assert!(error.to_string().contains("Authority"));
}
