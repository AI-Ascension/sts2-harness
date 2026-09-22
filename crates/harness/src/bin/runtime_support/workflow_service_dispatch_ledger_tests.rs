// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Acceptance for the served composition's durable receipt-ledger configuration.
//!
//! An unset variable must leave the served composition on session-lifetime receipts, so a
//! deployment that never opted in cannot acquire an undeclared durable surface. A set path must be
//! attached verbatim, and an explicitly empty value must be refused rather than read as "no store",
//! so a misconfigured deployment cannot silently lose the once-only guarantee across a restart.

use super::path_from_value;
use std::path::PathBuf;

#[test]
fn an_unset_variable_attaches_no_store() {
    assert_eq!(path_from_value(None), Ok(None));
}

#[test]
fn a_set_path_is_attached_verbatim() {
    assert_eq!(
        path_from_value(Some("/var/lib/sts2/dispatch-ledger.json".to_owned())),
        Ok(Some(PathBuf::from("/var/lib/sts2/dispatch-ledger.json")))
    );
}

#[test]
fn an_empty_or_blank_value_is_refused() {
    assert!(path_from_value(Some(String::new())).is_err());
    assert!(path_from_value(Some("   ".to_owned())).is_err());
}
