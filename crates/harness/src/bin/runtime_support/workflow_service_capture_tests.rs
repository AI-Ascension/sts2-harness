// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Acceptance for the served composition's configured capture surface.
//!
//! An unset surface must keep the recording ring the merged served composition already attaches, so a
//! deployment that never opted in keeps recording exactly as before. A recognised mode must be
//! attached with the bound it was resolved with, and every unrecognised, empty or out-of-range value
//! must be refused rather than silently downgraded, so a misconfigured deployment cannot lose a
//! boundary it believes it recorded.

use super::{CaptureConfiguration, configuration_from_value};
use sts2_harness::{CaptureMode, MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS};

/// The decision for one observed mode/records/bytes surface.
fn decision(
    mode: Option<&str>,
    records: Option<&str>,
    bytes: Option<&str>,
) -> Result<CaptureConfiguration, String> {
    configuration_from_value(
        mode.map(str::to_owned),
        records.map(str::to_owned),
        bytes.map(str::to_owned),
    )
}

#[test]
fn an_unset_surface_keeps_the_recording_ring_the_merged_composition_attaches() {
    let configuration = decision(None, None, None).expect("the unset surface is the default");
    assert_eq!(configuration.mode, CaptureMode::Memory);
    assert_eq!(configuration.max_records, MAX_CAPTURE_RECORDS);
    assert_eq!(configuration.max_content_bytes, MAX_CAPTURE_BYTES);
    assert!(configuration.sink().is_ok());
}

#[test]
fn metadata_records_the_ring_without_content() {
    let configuration = decision(Some("metadata"), None, None).expect("metadata mode");
    assert_eq!(configuration.mode, CaptureMode::Metadata);
    assert!(configuration.sink().is_ok());
}

#[test]
fn an_explicit_off_attaches_the_sink_that_cannot_record() {
    let configuration = decision(Some("off"), None, None).expect("off mode");
    assert_eq!(configuration.mode, CaptureMode::Off);
    assert!(configuration.sink().is_ok());
}

#[test]
fn configured_bounds_are_honored_up_to_the_module_maxima() {
    let configuration = decision(Some("memory"), Some("4"), Some("2048")).expect("in-range bounds");
    assert_eq!(configuration.max_records, 4);
    assert_eq!(configuration.max_content_bytes, 2048);
    assert!(configuration.sink().is_ok());

    let at_maximum = decision(
        Some("memory"),
        Some(&MAX_CAPTURE_RECORDS.to_string()),
        Some(&MAX_CAPTURE_BYTES.to_string()),
    )
    .expect("the module maxima are valid bounds");
    assert_eq!(at_maximum.max_records, MAX_CAPTURE_RECORDS);
    assert_eq!(at_maximum.max_content_bytes, MAX_CAPTURE_BYTES);
}

#[test]
fn an_unknown_or_empty_mode_is_refused_rather_than_downgraded() {
    for value in ["", "   ", "Memory", "disk", "private", "none", "true"] {
        assert!(
            decision(Some(value), None, None).is_err(),
            "mode {value:?} must be refused"
        );
    }
}

#[test]
fn an_out_of_range_or_non_numeric_bound_is_refused() {
    for value in ["0", "-1", "129", "not-a-number", "1.5", ""] {
        assert!(
            decision(Some("memory"), Some(value), None).is_err(),
            "records bound {value:?} must be refused"
        );
    }
    let over = (MAX_CAPTURE_BYTES + 1).to_string();
    assert!(decision(Some("metadata"), None, Some(&over)).is_err());
}

#[test]
fn a_bound_combined_with_off_is_refused() {
    assert!(decision(Some("off"), Some("4"), None).is_err());
    assert!(decision(Some("off"), None, Some("2048")).is_err());
}
