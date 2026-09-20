// SPDX-License-Identifier: MIT

//! The read port admits a caller, and keeps admitting them on every later read.
//!
//! A grant is an admission decision rather than a durable capability, so the source still has to
//! serve the owner the grant was made for. The port repeats every rule the store would apply, and
//! refuses a request before a source is asked rather than letting a source answer something the
//! caller was never allowed to ask.

#![allow(clippy::expect_used, dead_code)]

use std::cell::Cell;

use sts2_harness::semantic_history::{
    SemanticHistoryAgentPort, SemanticHistoryAuthority, SemanticHistoryError, SemanticHistoryQuery,
    SemanticHistorySourcePort, SemanticHistorySourceRequest,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[path = "support/semantic_history_port_doubles.rs"]
mod port_doubles;

use port_doubles::*;

#[test]
fn a_granted_port_serves_a_page_and_a_summary() {
    let store = fixture::store_with_events(2);
    let port = granted(&store);
    assert_eq!(port.authority(), SemanticHistoryAuthority::HarnessOwned);
    let page = port
        .page(&SemanticHistoryQuery::branch(fixture::ROOT, 10))
        .expect("page");
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].input.event_id, "event_1");
    let summary = port.summary(fixture::ROOT).expect("summary");
    assert_eq!(summary.total, 2);
    assert_eq!(summary.first_sequence, Some(1));
}

#[test]
fn a_caller_that_asks_for_its_own_authority_is_refused_a_port() {
    let store = fixture::store_with_events(0);
    // A caller that wants to read storage itself is refused here rather than handed a port.
    assert_eq!(
        SemanticHistoryAgentPort::grant(
            &store,
            SemanticHistoryAuthority::NotGranted,
            fixture::binding()
        )
        .err()
        .expect("no port for a non-owner"),
        SemanticHistoryError::Authority
    );
}

#[test]
fn a_port_granted_for_another_run_scope_is_refused() {
    let store = fixture::store_with_events(0);
    assert_eq!(
        SemanticHistoryAgentPort::grant(
            &store,
            SemanticHistoryAuthority::HarnessOwned,
            fixture::binding_other_run()
        )
        .err()
        .expect("another run"),
        SemanticHistoryError::Scope
    );
}

#[test]
fn a_port_granted_for_another_epoch_is_refused() {
    let store = fixture::store_with_events(0);
    assert_eq!(
        SemanticHistoryAgentPort::grant(
            &store,
            SemanticHistoryAuthority::HarnessOwned,
            fixture::binding_other_epoch()
        )
        .err()
        .expect("another epoch"),
        SemanticHistoryError::Epoch
    );
}

#[test]
fn a_grant_with_an_invalid_binding_is_refused_before_any_read() {
    let store = fixture::store_with_events(0);
    // A scope field that could be read as a path is refused before the scope is even compared.
    assert_eq!(
        SemanticHistoryAgentPort::grant(
            &store,
            SemanticHistoryAuthority::HarnessOwned,
            fixture::binding_non_opaque()
        )
        .err()
        .expect("non-opaque scope"),
        SemanticHistoryError::NonOpaqueIdentity("binding.run_id")
    );
}

#[test]
fn a_grant_that_is_no_longer_the_sources_scope_is_refused_on_its_next_read() {
    let source = DriftingSource {
        store: fixture::store_with_events(1),
        bindings: [fixture::binding(), fixture::binding_other_run()],
        current: Cell::new(0),
    };
    let port = SemanticHistoryAgentPort::grant(
        &source,
        SemanticHistoryAuthority::HarnessOwned,
        fixture::binding(),
    )
    .expect("grant");
    assert!(port.summary(fixture::ROOT).is_ok());
    source.current.set(1);
    // A grant is not a durable capability: the source must still serve the owner on the read.
    assert_eq!(
        port.summary(fixture::ROOT),
        Err(SemanticHistoryError::Scope)
    );
    assert_eq!(
        port.page(&SemanticHistoryQuery::branch(fixture::ROOT, 1)),
        Err(SemanticHistoryError::Scope)
    );
}

#[test]
fn a_grant_that_is_no_longer_the_sources_epoch_is_refused_on_its_next_read() {
    let source = DriftingSource {
        store: fixture::store_with_events(1),
        bindings: [fixture::binding(), fixture::binding_other_epoch()],
        current: Cell::new(0),
    };
    let port = SemanticHistoryAgentPort::grant(
        &source,
        SemanticHistoryAuthority::HarnessOwned,
        fixture::binding(),
    )
    .expect("grant");
    source.current.set(1);
    assert_eq!(
        port.summary(fixture::ROOT),
        Err(SemanticHistoryError::Epoch)
    );
}

#[test]
fn naming_storage_directly_is_refused_rather_than_answered() {
    let store = fixture::store_with_events(1);
    let port = granted(&store);
    assert_eq!(
        port.refuse_direct_storage("file:///var/history/run_0001"),
        Err(SemanticHistoryError::Authority)
    );
    assert_eq!(
        port.refuse_direct_storage("/var/history"),
        Err(SemanticHistoryError::Authority)
    );
}

#[test]
fn a_port_refuses_a_query_whose_branch_identity_is_not_opaque() {
    let store = fixture::store_with_events(1);
    let port = granted(&store);
    assert_eq!(
        port.page(&SemanticHistoryQuery::branch("/etc/passwd", 5)),
        Err(SemanticHistoryError::NonOpaqueIdentity("query.branch_id"))
    );
    assert_eq!(
        port.summary("../run_0001"),
        Err(SemanticHistoryError::NonOpaqueIdentity("branch_id"))
    );
}

#[test]
fn a_port_refuses_a_query_that_is_out_of_bounds() {
    let store = fixture::store_with_events(1);
    let port = granted(&store);
    assert_eq!(
        port.page(&SemanticHistoryQuery::branch(fixture::ROOT, 0)),
        Err(SemanticHistoryError::Bounds)
    );
    assert_eq!(
        port.page(&SemanticHistoryQuery::branch(fixture::ROOT, 4096)),
        Err(SemanticHistoryError::Bounds)
    );
}

#[test]
fn a_source_response_of_the_wrong_shape_is_refused_by_the_port() {
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
        port.page(&SemanticHistoryQuery::branch(fixture::ROOT, 1)),
        Err(SemanticHistoryError::Port)
    );
    let page_only = PageOnlySource {
        store: fixture::store_with_events(1),
        binding: fixture::binding(),
    };
    let port = SemanticHistoryAgentPort::grant(
        &page_only,
        SemanticHistoryAuthority::HarnessOwned,
        fixture::binding(),
    )
    .expect("grant");
    assert_eq!(port.summary(fixture::ROOT), Err(SemanticHistoryError::Port));
}

#[test]
fn reading_an_unknown_branch_is_refused_by_the_store_port() {
    let store = fixture::store_with_events(1);
    assert_eq!(
        store.read(&SemanticHistorySourceRequest::Summary {
            branch_id: "branch_absent".to_owned()
        }),
        Err(SemanticHistoryError::Branch)
    );
    assert_eq!(
        store.read(&SemanticHistorySourceRequest::Page {
            query: SemanticHistoryQuery::branch("branch_absent", 5),
            continuation: None,
        }),
        Err(SemanticHistoryError::Branch)
    );
    let port = granted(&store);
    assert_eq!(
        port.summary("branch_absent"),
        Err(SemanticHistoryError::Branch)
    );
}
