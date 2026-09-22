// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Acceptance for the file-backed durable prepared-dispatch ledger.
//!
//! The store is accepted only when it round-trips a committed image through the filesystem and
//! refuses to be mistaken for a committed one it is not: a missing file is the honest empty answer,
//! but an unreadable, malformed, mis-versioned or oversized file is a refusal rather than an empty
//! restart. A save that is told it is durable must also survive a replacement of the old image.

use super::*;
use crate::context_capture::{CaptureBoundary, DispatchLedger, DispatchOutcome, DispatchReceipt};
use std::fs;
use std::path::PathBuf;

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

fn image() -> DurableDispatchLedger {
    let mut ledger = DispatchLedger::new();
    ledger.record_receipt(receipt("exo.a"));
    ledger.record_receipt(receipt("exo.b"));
    ledger.mark_cancelled("exo.c");
    ledger.durable()
}

/// A per-test scratch directory removed when the test finishes.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sts2-dispatch-ledger-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create scratch directory");
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_saved_image_is_reloaded_by_a_fresh_port() {
    let scratch = Scratch::new("round-trip");
    let path = scratch.file("ledger.json");
    let committed = image();
    FileDispatchLedgerPort::open(&path)
        .save(&committed)
        .expect("the file port saves a valid image");

    let mut reloaded = FileDispatchLedgerPort::open(&path);
    assert_eq!(reloaded.path(), path.as_path());
    let loaded = reloaded
        .load()
        .expect("the committed image loads")
        .expect("the image is present");
    assert_eq!(loaded, committed);
    assert_eq!(
        loaded
            .restore()
            .expect("the reloaded image restores")
            .receipt_count(),
        2
    );
}

#[test]
fn a_port_without_a_written_image_answers_none() {
    let scratch = Scratch::new("absent");
    let mut port = FileDispatchLedgerPort::open(scratch.file("never-written.json"));
    assert_eq!(port.load(), Ok(None));
}

#[test]
fn an_unreadable_store_is_unavailable_rather_than_empty() {
    let scratch = Scratch::new("unreadable");
    // A directory where the image is expected cannot be read as an image; the image is therefore
    // unknown, and a restart must refuse it instead of assuming nothing was written.
    let directory = scratch.file("image");
    fs::create_dir(&directory).expect("create a directory at the image path");
    assert_eq!(
        FileDispatchLedgerPort::open(&directory).load(),
        Err(DispatchLedgerError::Unavailable)
    );
}

#[test]
fn a_malformed_or_mis_versioned_image_is_invalid() {
    let scratch = Scratch::new("invalid");
    let malformed = scratch.file("malformed.json");
    fs::write(&malformed, b"{ not json").expect("write malformed bytes");
    assert_eq!(
        FileDispatchLedgerPort::open(&malformed).load(),
        Err(DispatchLedgerError::Invalid)
    );

    let mis_versioned = scratch.file("mis-versioned.json");
    fs::write(
        &mis_versioned,
        br#"{"schema":"ascension.dispatch-ledger.v2","receipts":[],"cancelled":[]}"#,
    )
    .expect("write a future schema");
    assert_eq!(
        FileDispatchLedgerPort::open(&mis_versioned).load(),
        Err(DispatchLedgerError::Invalid)
    );

    let unknown_field = scratch.file("unknown-field.json");
    fs::write(
        &unknown_field,
        br#"{"schema":"ascension.dispatch-ledger.v1","receipts":[],"cancelled":[],"extra":1}"#,
    )
    .expect("write an unknown field");
    assert_eq!(
        FileDispatchLedgerPort::open(&unknown_field).load(),
        Err(DispatchLedgerError::Invalid)
    );
}

#[test]
fn an_oversized_image_is_invalid() {
    let scratch = Scratch::new("oversized");
    let path = scratch.file("oversized.json");
    let file = fs::File::create(&path).expect("create the oversized image");
    file.set_len(MAX_DURABLE_DISPATCH_LEDGER_BYTES as u64 + 1)
        .expect("extend the image past the bound");
    assert_eq!(
        FileDispatchLedgerPort::open(&path).load(),
        Err(DispatchLedgerError::Invalid)
    );
}

#[test]
fn an_invalid_image_is_refused_before_anything_is_written() {
    let scratch = Scratch::new("refuse");
    let path = scratch.file("ledger.json");
    let mut invalid = image();
    invalid.schema = "ascension.dispatch-ledger.v2".to_owned();
    assert_eq!(
        FileDispatchLedgerPort::open(&path).save(&invalid),
        Err(DispatchLedgerError::Invalid)
    );
    assert!(!path.exists(), "a refused save writes no image");
}

#[test]
fn a_second_save_replaces_the_first_image_and_leaves_no_staging_file() {
    let scratch = Scratch::new("replace");
    let path = scratch.file("ledger.json");
    FileDispatchLedgerPort::open(&path)
        .save(&image())
        .expect("save the first image");

    let mut replacement = DispatchLedger::new();
    replacement.record_receipt(receipt("exo.z"));
    let committed = replacement.durable();
    FileDispatchLedgerPort::open(&path)
        .save(&committed)
        .expect("replace the image");

    assert_eq!(
        FileDispatchLedgerPort::open(&path).load(),
        Ok(Some(committed))
    );
    let entries: Vec<_> = fs::read_dir(&scratch.0)
        .expect("read the scratch directory")
        .map(|entry| {
            entry
                .expect("read a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(entries, vec!["ledger.json".to_owned()]);
}

#[test]
fn a_save_creates_the_parent_directory_it_was_given() {
    let scratch = Scratch::new("nested");
    let path = scratch.file("nested/deeper/ledger.json");
    FileDispatchLedgerPort::open(&path)
        .save(&image())
        .expect("save into a nested path");
    assert_eq!(
        FileDispatchLedgerPort::open(&path).load(),
        Ok(Some(image()))
    );
}
