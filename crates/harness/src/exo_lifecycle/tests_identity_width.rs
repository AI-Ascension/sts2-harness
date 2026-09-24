// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

//! The request-level identity width, held to the published wire bound (issue #458, ADR 0077).
//!
//! `sts2.exo-bridge-wire-v1` binds `decision_request.model_execution_id` and `.state_id` to
//! `$defs/id` (`maxLength` 512) and the protocol validator admits the same width. The lifecycle
//! manifest refused both at 128, so a host that followed the published schema was refused with
//! `LifecycleError::Invalid` before dispatch. These cases pin the request-level fields to the
//! published width while the envelope/control fields stay at their own 128-byte bound.

use super::*;
use crate::exo_lifecycle::types;

#[test]
fn request_level_identity_is_admitted_up_to_the_published_wire_width() {
    let fixture = Fixture::new();
    let mut manifest = fixture.manifest.clone();
    manifest.execution_id = "e".repeat(MAX_WIRE_ID_BYTES);
    manifest.authority.state_id = "s".repeat(MAX_WIRE_ID_BYTES);
    assert_eq!(
        manifest.validate(),
        Ok(()),
        "the published wire width must be admitted"
    );
}

#[test]
fn one_byte_past_the_wire_width_is_refused_for_each_request_level_field() {
    let fixture = Fixture::new();
    let mut manifest = fixture.manifest.clone();
    manifest.execution_id = "e".repeat(MAX_WIRE_ID_BYTES + 1);
    assert_eq!(manifest.validate(), Err(LifecycleError::Invalid));
    manifest.execution_id = "e".repeat(MAX_WIRE_ID_BYTES);
    manifest.authority.state_id = "s".repeat(MAX_WIRE_ID_BYTES + 1);
    assert_eq!(manifest.validate(), Err(LifecycleError::Invalid));
}

/// The band that the derived-prefix arithmetic used to refuse with a different error vocabulary.
#[test]
fn the_previously_unsettleable_band_is_now_admitted_as_a_whole() {
    let fixture = Fixture::new();
    for width in [109usize, 110, 111, 127, 128, 129, 256, 511, 512] {
        let mut manifest = fixture.manifest.clone();
        manifest.execution_id = "e".repeat(width);
        manifest.authority.state_id = "s".repeat(width);
        assert_eq!(
            manifest.validate(),
            Ok(()),
            "the request-level identity is admitted at {width} bytes"
        );
    }
}

/// Envelope/control identities keep their own 128-byte bound: only the two request-level fields
/// moved, so the published `$defs/control_id` contract is untouched.
#[test]
fn envelope_and_control_identities_stay_at_their_own_bound() {
    let fixture = Fixture::new();
    let mut manifest = fixture.manifest.clone();
    for control in [
        |m: &mut InvocationManifest| m.request_id = "r".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.host_turn_id = "t".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.model_revision = "v".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.authority.lease_id = "l".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.binding_id = "b".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.prepared_id = "p".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.operation_id = "o".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.reservation_id = "s".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.episode_attempt_id = "a".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.trajectory_id = "j".repeat(MAX_ID_BYTES + 1),
        |m: &mut InvocationManifest| m.provider_attempt_id = "c".repeat(MAX_ID_BYTES + 1),
    ] {
        let mut candidate = manifest.clone();
        control(&mut candidate);
        assert_eq!(candidate.validate(), Err(LifecycleError::Invalid));
    }
    manifest.request_id = "r".repeat(MAX_ID_BYTES);
    assert_eq!(manifest.validate(), Ok(()));
}

/// The two widths are separate predicates, so the request-level one cannot be widened by editing
/// the internal one, and the charset is shared.
#[test]
fn the_two_width_predicates_agree_on_grammar_and_differ_only_in_width() {
    assert!(types::wire_id("e/v1"));
    assert!(types::id("e/v1"));
    assert!(!types::wire_id(""));
    assert!(!types::id(""));
    assert!(!types::wire_id("e v1"));
    assert!(!types::id("e v1"));
    assert!(types::wire_id(&"e".repeat(MAX_WIRE_ID_BYTES)));
    assert!(!types::wire_id(&"e".repeat(MAX_WIRE_ID_BYTES + 1)));
    assert!(types::id(&"e".repeat(MAX_ID_BYTES)));
    assert!(!types::id(&"e".repeat(MAX_ID_BYTES + 1)));
}
