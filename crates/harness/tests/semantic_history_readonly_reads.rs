// SPDX-License-Identifier: MIT

//! What a granted port can walk once it is admitted: pages, continuations and explanations.
//!
//! A port that could not spend a continuation would answer the first page forever, and a port
//! that widened the caller's own traversal bound would report a complete answer to a question
//! that asked for a bounded one. Each answer arm must match the request arm.

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryAgentPort, SemanticHistoryAuthority, SemanticHistoryError, SemanticHistoryKind,
    SemanticHistoryQuery, SemanticHistoryTraversalLimits,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[path = "support/semantic_history_port_doubles.rs"]
mod port_doubles;

use port_doubles::*;

#[test]
fn a_granted_port_follows_a_multi_page_read_to_its_end() {
    let store = fixture::store_with_events(3);
    let port = granted(&store);
    let query = SemanticHistoryQuery::branch(fixture::ROOT, 2);
    let first = port.page(&query).expect("first page");
    assert_eq!(first.events.len(), 2);
    let continuation = first.continuation.clone().expect("continuation");
    // A port that could not spend a continuation would answer the first page forever, so the
    // continuation has to travel through the port rather than stopping at the source.
    let second = port
        .page_from(&query, Some(&continuation))
        .expect("second page");
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].input.event_id, "event_3");
    assert!(second.continuation.is_none());
}

#[test]
fn a_granted_port_explains_a_stated_chain() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::ChoiceMade,
        1,
    );
    fixture::append_linked(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::ChoiceMade,
        2,
        "event_1",
    );
    let port = granted(&store);
    let explanation = port
        .explain(
            fixture::ROOT,
            "event_2",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    assert_eq!(explanation.traversal.links.len(), 1);
    assert_eq!(explanation.traversal.links[0].from_event_id, "event_1");
}

#[test]
fn a_granted_port_refuses_a_traversal_beyond_its_bounds_and_an_unknown_event() {
    let store = fixture::store_with_events(1);
    let port = granted(&store);
    assert_eq!(
        port.explain(
            fixture::ROOT,
            "event_1",
            SemanticHistoryTraversalLimits {
                max_depth: 0,
                max_visits: 1,
            }
        ),
        Err(SemanticHistoryError::Bounds)
    );
    assert_eq!(
        port.explain(
            fixture::ROOT,
            "event_absent",
            SemanticHistoryTraversalLimits::bounded()
        ),
        Err(SemanticHistoryError::Causality)
    );
    // The event identity is checked before the source is asked, so a caller cannot name a host path.
    assert_eq!(
        port.explain(
            "/etc/passwd",
            "event_1",
            SemanticHistoryTraversalLimits::bounded()
        ),
        Err(SemanticHistoryError::NonOpaqueIdentity("branch_id"))
    );
}

#[test]
fn a_source_that_answers_with_another_shape_is_refused() {
    // The response arm must match the request arm: a source that answers an explanation request with
    // a summary would otherwise hand the caller a different answer than the one they asked for.
    let summary_only = SummaryOnlySource {
        binding: fixture::binding(),
    };
    let port = SemanticHistoryAgentPort::grant(
        &summary_only,
        SemanticHistoryAuthority::HarnessOwned,
        fixture::binding(),
    )
    .expect("grant");
    assert_eq!(
        port.explain(
            fixture::ROOT,
            "event_1",
            SemanticHistoryTraversalLimits::bounded()
        ),
        Err(SemanticHistoryError::Port)
    );
}

#[test]
fn a_granted_port_walks_no_further_than_the_caller_asked() {
    let mut store = fixture::store();
    for sequence in 1..=5_u64 {
        let parent = format!("event_{}", sequence - 1);
        if sequence == 1 {
            fixture::append_plain(
                &mut store,
                fixture::ROOT,
                "event_1",
                SemanticHistoryKind::ChoiceMade,
                1,
            );
        } else {
            fixture::append_linked(
                &mut store,
                fixture::ROOT,
                &format!("event_{sequence}"),
                SemanticHistoryKind::ChoiceMade,
                sequence,
                &parent,
            );
        }
    }
    let port = granted(&store);
    let tighter = port
        .explain(
            fixture::ROOT,
            "event_5",
            SemanticHistoryTraversalLimits {
                max_depth: 1,
                max_visits: 16,
            },
        )
        .expect("explain");
    // A port that widened the caller's own bound to the hard maximum would walk the whole chain and
    // report a complete answer to a question that asked for a bounded one.
    assert_eq!(tighter.traversal.links.len(), 1);
    assert!(tighter.traversal.truncated);
    let wider = port
        .explain(
            fixture::ROOT,
            "event_5",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    assert_eq!(wider.traversal.links.len(), 4);
}

#[test]
fn a_request_this_port_would_refuse_never_reaches_the_source() {
    // A port that forwarded a malformed question would make the source responsible for the port's
    // own vocabulary, and a source that answered it anyway would answer something nobody asked.
    let recording = RecordingSource {
        binding: fixture::binding(),
        asked: std::cell::RefCell::new(Vec::new()),
    };
    let port = SemanticHistoryAgentPort::grant(
        &recording,
        SemanticHistoryAuthority::HarnessOwned,
        fixture::binding(),
    )
    .expect("grant");
    assert_eq!(
        port.explain(
            fixture::ROOT,
            "event_1",
            SemanticHistoryTraversalLimits {
                max_depth: 0,
                max_visits: 1,
            }
        ),
        Err(SemanticHistoryError::Bounds)
    );
    assert_eq!(
        port.explain(
            fixture::ROOT,
            "/etc/passwd",
            SemanticHistoryTraversalLimits::bounded()
        ),
        Err(SemanticHistoryError::NonOpaqueIdentity("event_id"))
    );
    assert_eq!(
        port.page(&SemanticHistoryQuery::branch(fixture::ROOT, 0)),
        Err(SemanticHistoryError::Bounds)
    );
    assert!(recording.asked.borrow().is_empty());
}
