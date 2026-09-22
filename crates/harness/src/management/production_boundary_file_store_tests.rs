// SPDX-License-Identifier: MIT

//! Served acceptance for the *file-backed* durable prepared-dispatch receipt store.
//!
//! The in-memory restart proves the ledger protocol; this proves the concrete file store an
//! operator attaches is wired to the served session, so a restart that reads the committed file
//! refuses a second write instead of starting from an empty ledger.

use super::boundary_durable_tests::{dispatch_id, served_session};
use super::boundary_tests::{ObservingSink, SELECTED_ITEMS};
use super::managed_render_tests::{render_test_session_with_state, selected_limits};
use super::*;
use crate::context_capture::FileDispatchLedgerPort;
use std::sync::atomic::Ordering;

/// A served boundary ledger restored from the file image at `path`.
fn restored(path: &std::path::Path) -> ServedBoundaryLedger {
    ServedBoundaryLedger::restored(ServedDispatchLedger::new(Box::new(
        FileDispatchLedgerPort::open(path),
    )))
    .expect("the file store restores the committed ledger")
}

#[test]
fn served_boundary_restart_through_a_file_store_refuses_a_second_write() {
    let root = std::env::temp_dir().join(format!("sts2-served-ledger-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create the scratch directory");
    let path = root.join("dispatch-ledger.json");
    let id = dispatch_id();

    let (mut first, state, first_exchanges, config) = served_session();
    first.boundary = restored(&path);
    first
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the first served decision is admitted");
    assert_eq!(first_exchanges.load(Ordering::SeqCst), 1);
    drop(first);

    // The committed file carries the receipt the first session wrote, so the restarted session
    // rebuilds its ledger from that image rather than from an empty one.
    let committed = FileDispatchLedgerPort::open(&path)
        .load()
        .expect("the committed file is readable")
        .expect("the first session committed an image");
    assert_eq!(committed.receipts.len(), 1);
    assert_eq!(committed.receipts[0].dispatch_id, id);

    let (mut second, _, second_exchanges, _) = render_test_session_with_state(
        Arc::clone(&state),
        config,
        selected_limits(SELECTED_ITEMS),
        Default::default(),
        None,
    );
    second.boundary_capture = BoundaryCaptureSink::new(Box::new(ObservingSink::new()));
    second.boundary = restored(&path);

    let error = second
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("the file receipt forbids a second write of the same approval");
    assert_eq!(error.code, "prepared_boundary_already_recorded");
    assert_eq!(second_exchanges.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&root);
}
