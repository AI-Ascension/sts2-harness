// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/prepared_input_budget_fixtures.rs"]
mod fixture;

use fixture::{base_request, capabilities, new_ledger, optional_entry, pinned_entry, pins};
use sts2_harness::context_memory::*;
use sts2_harness::effective_limits::UnavailableReason;

#[test]
fn exact_boundary_and_one_over_include_wrappers_schema_tools_and_output_reserve() {
    let capabilities = capabilities();
    let mut request = base_request();
    request.limits.output_reserve_bytes = 100;
    let protected = request.protected_bytes();
    request.limits.whole_input_byte_bound = protected + 100 + 32;
    let exact = request.limits.whole_input_byte_bound;

    let mut ledger = new_ledger();
    let budget = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-exact",
        )
        .expect("exact boundary prepares");
    assert_eq!(budget.input_bytes, protected + 32);
    assert_eq!(budget.whole_bytes_including_reserve, protected + 32 + 100);
    assert!(budget.whole_bytes_including_reserve <= exact);
    assert!(budget.exclusions.is_empty());
    assert_eq!(budget.output_reserve_bytes, 100);

    request.limits.whole_input_byte_bound = exact - 1;
    let mut ledger = new_ledger();
    let one_over = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-one-over",
        )
        .expect("one over still prepares by evicting optional content");
    assert_eq!(one_over.input_bytes, protected + 16);
    assert_eq!(one_over.exclusions.len(), 1);
    assert_eq!(one_over.exclusions[0].reason, "whole_input_budget");

    // The output reserve is part of the same window: a bound that only fits the input fails, and
    // the refusal arithmetic counts the reserve beside the protected components.
    request.limits.whole_input_byte_bound = protected + 32;
    let mut ledger = new_ledger();
    match request.prepare(
        PreparedInputAdmission::Capability(&capabilities),
        &mut ledger,
        "res-no-reserve-room",
    ) {
        Err(PreparedBudgetError::ProtectedOverflow {
            field,
            requested,
            effective,
            overflow_bytes,
        }) => {
            assert_eq!(field, "max_job_input_bytes");
            // The refusal counts the reserve beside the protected components: 75 protected plus a
            // 100-byte reserve cannot fit a 107-byte bound.
            assert_eq!(requested, protected + 100);
            assert_eq!(effective, protected + 32);
            // The 100-byte reserve leaves 7 bytes of headroom against 75 protected bytes.
            assert_eq!(overflow_bytes, 68);
        }
        other => panic!("unexpected outcome {other:?}"),
    }
    assert_eq!(ledger.reserved_bytes(), 0);

    // A reserve larger than the whole-input bound cannot be admitted at all.
    request.limits.output_reserve_bytes = 8192;
    request.limits.whole_input_byte_bound = 4096;
    let mut ledger = new_ledger();
    assert_eq!(
        request
            .prepare(
                PreparedInputAdmission::Capability(&capabilities),
                &mut ledger,
                "res-reserve-over-bound",
            )
            .expect_err("reserve exceeds the bound"),
        PreparedBudgetError::OutputReserveOverflow {
            requested: 8192,
            effective: 4096,
        }
    );
    assert_eq!(ledger.reserved_bytes(), 0);
}

#[test]
fn multibyte_unicode_is_counted_in_bytes_not_characters() {
    let capabilities = capabilities();
    let mut request = base_request();
    let crabs = "🦀".repeat(5);
    assert_eq!((crabs.len(), crabs.chars().count()), (20, 5));
    request.optional = vec![optional_entry("opt-crab", crabs.into_bytes())];
    // One five-character, twenty-byte entry under an eight-byte optional budget. Eight characters
    // would admit the crab and eight bytes do not, so the fixture discriminates byte accounting from
    // character accounting instead of consuming the whole limit either way.
    request.limits.optional_byte_budget = 8;
    request.limits.output_reserve_bytes = 8;
    let protected = request.protected_bytes();
    request.limits.whole_input_byte_bound = protected + 8 + 20;

    let mut ledger = new_ledger();
    let budget = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-unicode",
        )
        .expect("unicode fixture prepares");
    assert!(budget.selected_optional.is_empty());
    assert_eq!(budget.optional_rendered_bytes, 0);
    assert_eq!(budget.input_bytes, protected);
    assert_eq!(budget.exclusions.len(), 1);
    assert_eq!(budget.exclusions[0].entry_id, "opt-crab");
    assert_eq!(budget.exclusions[0].reason, "optional_byte_budget");
    // The entry's byte-valued field is its twenty bytes, not its five characters.
    assert_eq!(request.optional[0].reference().bytes, 20);

    // The same entry fits a twenty-byte optional budget exactly, and the admitted reference records
    // the byte length rather than the character count.
    request.limits.optional_byte_budget = 20;
    let mut ledger = new_ledger();
    let admitted = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-unicode-exact",
        )
        .expect("a twenty-byte budget admits the crab exactly");
    assert!(admitted.exclusions.is_empty());
    assert_eq!(admitted.selected_optional.len(), 1);
    assert_eq!(admitted.selected_optional[0].entry_id, "opt-crab");
    assert_eq!(admitted.selected_optional[0].bytes, 20);
    assert_eq!(admitted.optional_rendered_bytes, 20);
    assert_eq!(admitted.input_bytes, protected + 20);
}

#[test]
fn optional_eviction_is_stable_across_candidate_order() {
    let capabilities = capabilities();
    let mut forward = base_request();
    forward.limits.optional_byte_budget = 24;
    forward.optional = vec![
        optional_entry("opt-1", vec![b'a'; 16]),
        optional_entry("opt-2", vec![b'b'; 16]),
        optional_entry("opt-3", vec![b'c'; 16]),
    ];
    let mut reversed = forward.clone();
    reversed.optional.reverse();

    let mut ledger = new_ledger();
    let first = forward
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-forward",
        )
        .expect("forward prepares");
    let second = reversed
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-reversed",
        )
        .expect("reversed prepares");
    assert_eq!(first.selected_optional, second.selected_optional);
    assert_eq!(first.exclusions, second.exclusions);
    assert_eq!(
        first.optional_rendered_bytes,
        second.optional_rendered_bytes
    );
    assert_eq!(first.prepared_input_sha256, second.prepared_input_sha256);
    assert_eq!(first.selected_optional[0].entry_id, "opt-1");
}

#[test]
fn mandatory_and_pinned_overflow_reject_before_any_provider_reservation() {
    let capabilities = capabilities();

    let mut request = base_request();
    request.mandatory = vec![b'm'; 4096];
    request.limits.whole_input_byte_bound = 1024;
    let mut ledger = new_ledger();
    let outcome = request.prepare(
        PreparedInputAdmission::Capability(&capabilities),
        &mut ledger,
        "res-mandatory-overflow",
    );
    // Only a prepared budget can dispatch a provider call, so a refusal yields zero calls.
    assert_eq!(usize::from(outcome.is_ok()), 0);
    let refusal = outcome.expect_err("mandatory overflow is refused");
    assert!(matches!(
        refusal,
        PreparedBudgetError::ProtectedOverflow {
            field: "max_job_input_bytes",
            effective: 1024,
            overflow_bytes,
            ..
        } if overflow_bytes > 0
    ));

    let mut request = base_request();
    request.pinned = vec![pinned_entry("pin-big", 8000)];
    let mut pinned_ledger = new_ledger();
    let outcome = request.prepare(
        PreparedInputAdmission::Capability(&capabilities),
        &mut pinned_ledger,
        "res-pinned-overflow",
    );
    assert_eq!(usize::from(outcome.is_ok()), 0);
    let refusal = outcome.expect_err("pinned overflow is refused");
    assert!(matches!(
        refusal,
        PreparedBudgetError::ProtectedOverflow { .. }
    ));

    for ledger in [ledger, pinned_ledger] {
        assert_eq!(ledger.reserved_bytes(), 0);
        assert_eq!(ledger.active_jobs(), 0);
    }
}

#[test]
fn token_measurements_stay_qualified_and_never_present_bytes_as_tokens() {
    let capabilities = capabilities();
    let mut request = base_request();
    let mut ledger = new_ledger();
    let budget = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-unavailable",
        )
        .expect("unavailable measurement still prepares");
    assert_eq!(budget.tokens(), None);
    assert_eq!(budget.budget_status, "bounded_unknown_tokens");
    let value = serde_json::to_value(&budget).expect("record value");
    assert!(value["measurement"]["tokens"].is_null());
    assert_eq!(value["measurement"]["provenance"], "unavailable");
    assert!(budget.measurement.describe().contains("unavailable"));

    request.measurement = TokenMeasurement::heuristic(
        MeasurementScope::PreparedInput,
        14,
        "bytes-over-four-approximation",
    );
    let mut ledger = new_ledger();
    let heuristic = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-heuristic",
        )
        .expect("heuristic measurement prepares");
    assert_eq!(heuristic.tokens(), Some(14));
    assert!(!heuristic.measurement.is_exact());
    assert_eq!(heuristic.budget_status, "bounded_heuristic_tokens");
    assert!(heuristic.measurement.describe().contains("heuristic"));

    request.measurement =
        TokenMeasurement::local_tokenizer(MeasurementScope::PreparedInput, 15, "tokenizer-fixture");
    let mut ledger = new_ledger();
    let measured = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-measured",
        )
        .expect("measured quantity prepares");
    assert!(measured.measurement.is_exact());
    assert_eq!(measured.budget_status, "measured_within_bound");

    let report =
        TokenMeasurement::provider_reported(MeasurementScope::ProviderTurn, 21, "provider-fixture");
    let attached = measured
        .with_provider_report(report)
        .expect("provider report");
    assert_eq!(attached.provider_report.expect("report").tokens(), Some(21));
    assert_eq!(attached.input_bytes, measured.input_bytes);
    assert!(
        measured
            .with_provider_report(TokenMeasurement::local_tokenizer(
                MeasurementScope::PreparedInput,
                1,
                "tokenizer-fixture"
            ))
            .is_err()
    );

    request.measurement =
        TokenMeasurement::provider_reported(MeasurementScope::PreparedInput, 0, "provider-fixture");
    let mut ledger = new_ledger();
    assert_eq!(
        request
            .prepare(
                PreparedInputAdmission::Capability(&capabilities),
                &mut ledger,
                "res-zero",
            )
            .expect_err("a measured quantity of zero is invalid"),
        PreparedBudgetError::InvalidMeasurement("tokens")
    );
}

#[test]
fn profile_admission_and_drift_reject_consistently() {
    let capabilities = capabilities();
    let mut request = base_request();
    request.limits.optional_byte_budget = 16_384;
    let mut ledger = new_ledger();
    match request.prepare(
        PreparedInputAdmission::Capability(&capabilities),
        &mut ledger,
        "res-inadmissible",
    ) {
        Err(PreparedBudgetError::ProfileInadmissible {
            field,
            requested,
            reason,
        }) => {
            assert_eq!(field, "optional_byte_budget");
            assert_eq!(requested, 16_384);
            assert_eq!(reason, UnavailableReason::EffectiveLimitExceeded);
        }
        other => panic!("unexpected outcome {other:?}"),
    }
    assert_eq!(ledger.reserved_bytes(), 0);

    let record = capabilities.effective_limit_record();
    request.limits.optional_byte_budget = 8192;
    let mut tampered = record.clone();
    tampered.rows[3].executable_ceiling = 4096;
    match request.prepare(
        PreparedInputAdmission::Authenticated {
            capabilities: &capabilities,
            record: &tampered,
        },
        &mut ledger,
        "res-tampered",
    ) {
        Err(PreparedBudgetError::ProfileInadmissible { reason, .. }) => {
            assert_eq!(reason, UnavailableReason::DescriptorTampered);
        }
        other => panic!("unexpected outcome {other:?}"),
    }

    let mut ledger = new_ledger();
    let budget = request
        .prepare(
            PreparedInputAdmission::Authenticated {
                capabilities: &capabilities,
                record: &record,
            },
            &mut ledger,
            "res-authenticated",
        )
        .expect("authenticated record admits");
    let mut drifted = pins("tokenizer-other");
    drifted.model_id = "model-other".to_owned();
    assert_eq!(
        budget.revalidate(&drifted),
        Err(PreparedBudgetError::ApprovalInvalidated(vec![
            PreparedInputDrift::Model,
            PreparedInputDrift::Tokenizer,
        ]))
    );
    budget
        .revalidate(&request.pins)
        .expect("unchanged identities revalidate");
    let round_trip: PreparedInputBudget =
        serde_json::from_str(&serde_json::to_string(&budget).expect("json")).expect("round trip");
    assert_eq!(round_trip, budget);
}
