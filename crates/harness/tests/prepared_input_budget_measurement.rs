// SPDX-License-Identifier: MIT

//! Read-path enforcement for the prepared-input token measurement (issue #381).
//!
//! A recorded budget is read back through `serde`, so the invariant that `tokens` is `None` exactly
//! when the provenance is `Unavailable` has to hold for a deserialized record and not only for a
//! validated constructor. Every fixture here is synthetic: no provider, host, game, or clock is
//! contacted.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/prepared_input_budget_fixtures.rs"]
mod fixture;

use fixture::{base_request, capabilities, new_ledger};
use sts2_harness::context_memory::*;

#[test]
fn legitimate_measurements_keep_their_provenance_through_the_read_path() {
    let legitimate = [
        TokenMeasurement::unavailable(MeasurementScope::PreparedInput),
        TokenMeasurement::heuristic(MeasurementScope::PreparedInput, 14, "bytes-over-four"),
        TokenMeasurement::local_tokenizer(MeasurementScope::ProviderTurn, 15, "tokenizer-fixture"),
        TokenMeasurement::provider_reported(MeasurementScope::Cumulative, 21, "provider-fixture"),
    ];
    for measurement in legitimate {
        let value = serde_json::to_value(&measurement).expect("measurement value");
        let read: TokenMeasurement = serde_json::from_value(value).expect("valid record reads");
        assert_eq!(read, measurement);
        assert_eq!(read.provenance(), measurement.provenance());
        assert_eq!(read.scope(), measurement.scope());
        assert_eq!(read.tokens(), measurement.tokens());
        assert_eq!(read.method(), measurement.method());
        assert_eq!(read.describe(), measurement.describe());
    }
    assert_eq!(
        TokenMeasurement::unavailable(MeasurementScope::PreparedInput).method(),
        "none"
    );
}

#[test]
fn crafted_measurement_records_are_rejected_by_the_read_path() {
    fn crafted(provenance: &str, tokens: serde_json::Value, method: &str) -> serde_json::Value {
        serde_json::json!({
            "provenance": provenance,
            "scope": "prepared_input",
            "tokens": tokens,
            "method": method,
        })
    }

    let unavailable_with_tokens = crafted("unavailable", serde_json::json!(9), "none");
    let error = serde_json::from_value::<TokenMeasurement>(unavailable_with_tokens)
        .expect_err("an absent provenance carrying a quantity is rejected");
    assert!(
        error
            .to_string()
            .contains("unavailable_provenance_with_tokens"),
        "{error}"
    );

    let unlabelled = crafted(
        "local_tokenizer",
        serde_json::Value::Null,
        "tokenizer-fixture",
    );
    let error = serde_json::from_value::<TokenMeasurement>(unlabelled)
        .expect_err("a quantity with no provenance to label it is rejected");
    assert!(
        error.to_string().contains("token measurement tokens"),
        "{error}"
    );

    let zero = crafted("local_tokenizer", serde_json::json!(0), "tokenizer-fixture");
    let error =
        serde_json::from_value::<TokenMeasurement>(zero).expect_err("a zero quantity is rejected");
    assert!(
        error.to_string().contains("token measurement tokens"),
        "{error}"
    );

    let unnamed = crafted("unavailable", serde_json::Value::Null, "");
    let error = serde_json::from_value::<TokenMeasurement>(unnamed)
        .expect_err("an empty method is rejected");
    assert!(
        error.to_string().contains("token measurement method"),
        "{error}"
    );

    let mut unknown_field = crafted("unavailable", serde_json::Value::Null, "none");
    unknown_field["tokens_bytes"] = serde_json::json!(9);
    assert!(serde_json::from_value::<TokenMeasurement>(unknown_field).is_err());
}

#[test]
fn a_recorded_budget_cannot_be_read_back_with_a_byte_count_as_tokens() {
    let capabilities = capabilities();
    let request = base_request();
    let mut ledger = new_ledger();
    let budget = request
        .prepare(
            PreparedInputAdmission::Capability(&capabilities),
            &mut ledger,
            "res-crafted-record",
        )
        .expect("fixture prepares");
    let mut value = serde_json::to_value(&budget).expect("record value");

    // The exact crafted shape: the recorded input's byte count presented as a token quantity beside
    // an absent provenance, which `PreparedInputBudget::tokens()` would previously report as
    // `Some(input_bytes)`.
    value["measurement"] = serde_json::json!({
        "provenance": "unavailable",
        "scope": "prepared_input",
        "tokens": budget.input_bytes,
        "method": "none",
    });
    let error = serde_json::from_value::<PreparedInputBudget>(value.clone())
        .expect_err("the recorded budget refuses the crafted measurement");
    assert!(
        error
            .to_string()
            .contains("unavailable_provenance_with_tokens"),
        "{error}"
    );

    value["measurement"] = serde_json::json!({
        "provenance": "provider_reported",
        "scope": "prepared_input",
        "tokens": null,
        "method": "provider-fixture",
    });
    let error = serde_json::from_value::<PreparedInputBudget>(value)
        .expect_err("a reported provenance with no quantity is refused");
    assert!(
        error.to_string().contains("token measurement tokens"),
        "{error}"
    );
}
