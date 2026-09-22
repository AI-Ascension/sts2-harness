// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Acceptance for the durable prepared-dispatch ledger image.
//!
//! The image is the only thing a restarted composition reads back, so it is accepted only when it
//! round-trips exactly and is refused whenever it could be mistaken for a committed image it is
//! not: an unknown schema version, a duplicate or mis-ordered receipt, a cancellation of a recorded
//! identity, a malformed identity or digest, a receipt that does not account for exactly one write,
//! and a write that claims bytes it never wrote are all refusals rather than repairs.

use super::*;
use crate::context_capture::{CaptureBoundary, DispatchOutcome};

fn digest(seed: u8) -> String {
    std::iter::repeat_n(char::from(b'a' + seed), 64).collect()
}

fn receipt(dispatch_id: &str) -> DispatchReceipt {
    DispatchReceipt {
        dispatch_id: dispatch_id.to_owned(),
        execution_id: "model-execution-1".to_owned(),
        attempt_id: Some("attempt-1".to_owned()),
        adapter_id: "exo".to_owned(),
        boundary: CaptureBoundary::ExoSessionRequest,
        approved_material_sha256: digest(0),
        manifest_sha256: digest(1),
        outcome: DispatchOutcome::WriteCompleted,
        written_bytes: 12,
        boundary_writes: 1,
        gameplay_effects: 1,
    }
}

fn ledger() -> DispatchLedger {
    let mut ledger = DispatchLedger::new();
    ledger.record_receipt(receipt("exo.a"));
    ledger.record_receipt(receipt("exo.b"));
    ledger.mark_cancelled("exo.c");
    ledger
}

fn captured(ledger: &DispatchLedger) -> DurableDispatchLedger {
    let image = ledger.durable();
    image
        .validate()
        .expect("a captured ledger is a valid image");
    image
}

fn first_receipt(image: &mut DurableDispatchLedger) -> &mut DispatchReceipt {
    match image.receipts.first_mut() {
        Some(receipt) => receipt,
        None => panic!("the fixture records a receipt"),
    }
}

#[test]
fn durable_image_round_trips_every_receipt_and_cancellation() {
    let committed = ledger();
    let image = captured(&committed);

    assert_eq!(image.schema, DURABLE_DISPATCH_LEDGER_SCHEMA);
    assert_eq!(image.receipts.len(), 2);
    assert_eq!(image.cancelled, vec!["exo.c".to_owned()]);

    let reloaded = image
        .restore()
        .expect("a valid image restores its receipts");
    assert_eq!(reloaded.receipt_count(), 2);
    assert_eq!(reloaded.receipt("exo.a"), Some(&receipt("exo.a")));
    assert_eq!(reloaded.receipt("exo.b"), Some(&receipt("exo.b")));
    assert!(reloaded.receipt("exo.c").is_none());
    assert!(reloaded.is_cancelled("exo.c"));
    assert!(!reloaded.is_cancelled("exo.a"));
}

#[test]
fn durable_image_re_encodes_to_the_bytes_it_was_decoded_from() {
    let committed = captured(&ledger());
    let encoded = serde_json::to_string(&committed).expect("an image encodes");
    let decoded: DurableDispatchLedger =
        serde_json::from_str(&encoded).expect("an encoded image decodes");

    assert_eq!(decoded, committed);
    assert_eq!(
        serde_json::to_string(&decoded).expect("a decoded image re-encodes"),
        encoded
    );
}

#[test]
fn durable_image_refuses_an_unrecognised_schema_or_an_unknown_field() {
    let mut future = captured(&ledger());
    future.schema = "ascension.dispatch-ledger.v2".to_owned();
    assert_eq!(future.validate(), Err(DispatchLedgerError::Invalid));
    assert_eq!(future.restore(), Err(DispatchLedgerError::Invalid));

    let encoded = serde_json::to_string(&captured(&ledger())).expect("an image encodes");
    let extended = encoded.replace(r#""schema":"#, r#""unexpected":true,"schema":"#);
    assert!(serde_json::from_str::<DurableDispatchLedger>(&extended).is_err());
}

#[test]
fn durable_image_refuses_a_duplicate_or_misordered_receipt() {
    let mut duplicated = captured(&ledger());
    duplicated.receipts.insert(1, receipt("exo.a"));
    assert_eq!(duplicated.validate(), Err(DispatchLedgerError::Invalid));

    let mut misordered = captured(&ledger());
    misordered.receipts.reverse();
    assert_eq!(misordered.validate(), Err(DispatchLedgerError::Invalid));
}

#[test]
fn durable_image_refuses_a_cancellation_that_is_recorded_duplicated_or_misordered() {
    // A receipt is authoritative for its identity, so it may not also be cancelled terminally.
    let mut contradictory = captured(&ledger());
    contradictory.cancelled = vec!["exo.a".to_owned()];
    assert_eq!(contradictory.validate(), Err(DispatchLedgerError::Invalid));

    let mut duplicated = captured(&ledger());
    duplicated.cancelled = vec!["exo.c".to_owned(), "exo.c".to_owned()];
    assert_eq!(duplicated.validate(), Err(DispatchLedgerError::Invalid));

    let mut misordered = captured(&ledger());
    misordered.cancelled = vec!["exo.d".to_owned(), "exo.c".to_owned()];
    assert_eq!(misordered.validate(), Err(DispatchLedgerError::Invalid));

    let mut malformed = captured(&ledger());
    malformed.cancelled = vec![String::new()];
    assert_eq!(malformed.validate(), Err(DispatchLedgerError::Invalid));
}

#[test]
fn durable_image_refuses_a_malformed_identity_or_digest() {
    let mutations: [fn(&mut DispatchReceipt); 6] = [
        |receipt| receipt.dispatch_id = String::new(),
        |receipt| receipt.execution_id = String::new(),
        |receipt| receipt.adapter_id = String::new(),
        |receipt| receipt.attempt_id = Some(String::new()),
        |receipt| receipt.approved_material_sha256 = digest(0).to_uppercase(),
        |receipt| receipt.manifest_sha256 = digest(1)[..63].to_owned(),
    ];
    for mutate in mutations {
        let mut image = captured(&ledger());
        mutate(first_receipt(&mut image));
        assert_eq!(image.validate(), Err(DispatchLedgerError::Invalid));
    }
}

#[test]
fn durable_image_refuses_a_receipt_that_does_not_account_for_exactly_one_write() {
    let mutations: [fn(&mut DispatchReceipt); 3] = [
        |receipt| receipt.boundary_writes = 2,
        |receipt| receipt.boundary_writes = 0,
        |receipt| receipt.gameplay_effects = 2,
    ];
    for mutate in mutations {
        let mut image = captured(&ledger());
        mutate(first_receipt(&mut image));
        assert_eq!(image.validate(), Err(DispatchLedgerError::Invalid));
    }
}

#[test]
fn durable_image_refuses_an_indeterminate_outcome_that_claims_written_bytes() {
    let mut image = captured(&ledger());
    first_receipt(&mut image).outcome = DispatchOutcome::Unknown;
    first_receipt(&mut image).written_bytes = 12;
    assert_eq!(image.validate(), Err(DispatchLedgerError::Invalid));
}

#[test]
fn durable_image_keeps_the_indeterminate_outcome_that_wrote_nothing() {
    // A lost reply is exactly this: one write attempt, an unknown outcome and no known bytes.
    let mut image = captured(&ledger());
    let receipt = first_receipt(&mut image);
    receipt.outcome = DispatchOutcome::Unknown;
    receipt.written_bytes = 0;
    receipt.gameplay_effects = 0;

    assert_eq!(image.validate(), Ok(()));
    assert_eq!(
        image
            .restore()
            .expect("the indeterminate image restores")
            .receipt("exo.a")
            .map(|receipt| receipt.outcome),
        Some(DispatchOutcome::Unknown)
    );
}

#[test]
fn durable_image_refuses_a_wire_label_it_does_not_recognise() {
    assert_eq!(
        serde_json::from_str::<DispatchOutcome>(r#""write_completed""#).expect("a known outcome"),
        DispatchOutcome::WriteCompleted
    );
    assert!(serde_json::from_str::<DispatchOutcome>(r#""delivered""#).is_err());

    assert_eq!(
        serde_json::from_str::<CaptureBoundary>(r#""adapter.http_body""#)
            .expect("a known boundary"),
        CaptureBoundary::HttpBody
    );
    assert!(serde_json::from_str::<CaptureBoundary>(r#""adapter.socket""#).is_err());

    // The compatibility variant serializes to the canonical boundary, so no image carries it.
    assert_eq!(
        serde_json::to_string(&CaptureBoundary::ProviderRequest).expect("a boundary encodes"),
        r#""adapter.http_body""#
    );
}

#[test]
fn in_memory_port_reloads_the_image_it_saved_and_the_noop_port_never_persists_one() {
    let mut port = InMemoryDispatchLedgerPort::default();
    assert_eq!(port.load(), Ok(None));

    let image = captured(&ledger());
    port.save(&image)
        .expect("the in-memory port saves a valid image");
    assert_eq!(port.load(), Ok(Some(image.clone())));
    assert_eq!(port.image(), Some(&image));

    let mut noop = NoopDispatchLedgerPort;
    noop.save(&image).expect("the inert port accepts a save");
    assert_eq!(noop.load(), Ok(None));
}
