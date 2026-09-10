// SPDX-License-Identifier: MIT

use super::*;
use crate::episode::coop_receipt_query::{
    ReceiptQueryActionKind, ReceiptQueryCoordinate, ReceiptQueryLocation,
};

fn identity() -> Result<ReceiptQueryIdentity, super::super::identity::ReceiptQueryIdentityError> {
    ReceiptQueryIdentity::new(
        "op:run-17:0001",
        ReceiptQueryActionKind::PlayCard,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "session-native-17",
        "run-17",
        ReceiptQueryLocation::new(1, Some(42), Some(ReceiptQueryCoordinate::new(3, 5))),
        "peer-1",
        "native-host-a",
        "epoch-9",
        17,
        17,
        vec!["peer-1".into(), "peer-2".into()],
    )
}

#[test]
fn frozen_settled_response_is_retained_evidence() -> Result<(), String> {
    let value: Value = serde_json::from_str(include_str!(
        "../../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-settled.json"
    ))
    .map_err(|error| error.to_string())?;
    let identity = identity().map_err(|error| error.to_string())?;
    let result = ReceiptQueryResult::from_value(
        &value,
        &identity,
        "corr:17:1",
        "instance-1",
        "session-native-17",
        "lease-1",
        9,
    )
    .map_err(|error| error.to_string())?;
    if result.status() != ReceiptQueryStatus::Settled
        || result.receipt().and_then(ReceiptQueryReceipt::effect_id)
            != Some("effect:op:run-17:0001")
    {
        return Err("settled retained receipt was not parsed".into());
    }
    Ok(())
}

#[test]
fn wire_parser_rejects_duplicate_and_reordered_members() -> Result<(), String> {
    let text = include_str!(
        "../../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-settled.json"
    );
    let identity = identity().map_err(|error| error.to_string())?;
    let parse = |body: &str| {
        ReceiptQueryResult::from_json(
            body,
            &identity,
            "corr:17:1",
            "instance-1",
            "session-native-17",
            "lease-1",
            9,
        )
    };
    parse(text).map_err(|error| error.to_string())?;

    let duplicate = text.replacen(
        "\"error_code\":null}\n",
        "\"error_code\":null,\"error_code\":null}\n",
        1,
    );
    if parse(&duplicate).is_ok() {
        return Err("duplicate response member was accepted".into());
    }

    let reordered = text.replacen(
        "{\"protocol_version\":\"coop-receipt-query-v1\",\"schema_digest\":\"3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c\",",
        "{\"schema_digest\":\"3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c\",\"protocol_version\":\"coop-receipt-query-v1\",",
        1,
    );
    if parse(&reordered).is_ok() {
        return Err("reordered response members were accepted".into());
    }
    Ok(())
}

#[test]
fn wire_parser_accepts_each_canonical_outcome() -> Result<(), String> {
    let identity = identity().map_err(|error| error.to_string())?;
    for (text, expected) in [
        (
            include_str!(
                "../../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-accepted.json"
            ),
            ReceiptQueryStatus::Accepted,
        ),
        (
            include_str!(
                "../../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-settled.json"
            ),
            ReceiptQueryStatus::Settled,
        ),
        (
            include_str!(
                "../../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-rejected.json"
            ),
            ReceiptQueryStatus::Rejected,
        ),
        (
            include_str!(
                "../../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-unknown.json"
            ),
            ReceiptQueryStatus::Unknown,
        ),
    ] {
        let result = ReceiptQueryResult::from_json(
            text,
            &identity,
            "corr:17:1",
            "instance-1",
            "session-native-17",
            "lease-1",
            9,
        )
        .map_err(|error| error.to_string())?;
        if result.status() != expected {
            return Err(format!("canonical outcome parsed as {:?}", result.status()));
        }
    }
    Ok(())
}

#[test]
fn wire_parser_rejects_uncertain_outcomes_without_error_code() -> Result<(), String> {
    let text = include_str!(
        "../../../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-unknown.json"
    );
    let identity = identity().map_err(|error| error.to_string())?;
    let parse = |body: &str| {
        ReceiptQueryResult::from_json(
            body,
            &identity,
            "corr:17:1",
            "instance-1",
            "session-native-17",
            "lease-1",
            9,
        )
    };

    let unknown_without_error = text.replacen(
        "\"error_code\":\"native_receipt_query_cache_miss\"",
        "\"error_code\":null",
        1,
    );
    if parse(&unknown_without_error).is_ok() {
        return Err("unknown response without error_code was accepted".into());
    }

    let recovery_without_error = text
        .replacen(
            "\"status\":\"unknown\"",
            "\"status\":\"recovery_required\"",
            1,
        )
        .replacen(
            "\"error_code\":\"native_receipt_query_cache_miss\"",
            "\"error_code\":null",
            1,
        );
    if parse(&recovery_without_error).is_ok() {
        return Err("recovery_required response without error_code was accepted".into());
    }
    Ok(())
}
