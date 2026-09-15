// SPDX-License-Identifier: MIT
//! Original synthetic fixtures, generated in test from public protocol field names.
use super::*;
use serde_json::json;

fn request() -> Value {
    json!({
        "protocol_version":"game-information-query-v1","schema_digest":SCHEMA_DIGEST,
        "provenance":{"artifact":"sts2-protocol/game-information-query-v1","source":"schemas/game-information-query-v1.schema.json","generator":"hand-authored"},
        "correlation_id":"synthetic-query","kind":"query_request","result":null,"capabilities":null,"error":null,
        "query":{"query_kind":"list","entity_kind":"card",
            "target":{"definition_ref":null,"instance_ref":null},
            "filters":{"display_name":null,"namespaced_ids":[],"definition_refs":[],"instance_ids":[]},
            "projection":"summary","detail_level":"summary","fields":["display_name"],
            "binding":{"mode":"static","content_manifest_id":"synthetic-manifest","locale":"en","visibility_scope":"public","instance_ref":null,"snapshot_ref":null},
            "parent_observation":null,"limits":{"page_items":4,"item_bytes":4096,"page_bytes":65536,"text_bytes":4096},"cursor":null}
    })
}

fn response(request: &Value) -> Result<Value, ValidationError> {
    let mut value = request.clone();
    value["kind"] = json!("query_response");
    value["result"] = json!({"read_only":true,"parent_observation":null,"result_generation":null,
        "page":{"items":[{"definition_ref":{"content_manifest_id":"synthetic-manifest","entity_kind":"card","namespaced_id":"synthetic:alpha","variant":null},"instance_ref":null,
            "fields":[{"name":"display_name","kind":"text","availability":"available","value":"Alpha","unit":null,"source":{"kind":"synthetic","ref":null},"reason":null}]}],
            "next_cursor":null,"cursor_binding":null,"final_page":true,"total_count_known":true,"total_count":1,"coverage":"complete",
            "ordering":{"key":"definition_ref","direction":"ascending","algorithm":"identity_bytes","deterministic":true},
            "limits":request["query"]["limits"]}});
    let page = &mut value["result"]["page"];
    page["accounting"] = json!({
        "item_count":1,"item_bytes":encoded_len(&page["items"][0])?,
        "payload_bytes":encoded_len(&page["items"])?,"page_bytes":encoded_len(page)?,"text_bytes":5
    });
    Ok(value)
}

#[test]
fn original_static_pair_and_closed_nested_schema() -> Result<(), ValidationError> {
    let request = request();
    let mut response = response(&request)?;
    validate_request(&request)?;
    validate_response(&request, &response)?;
    response["result"]["page"]["items"][0]["seed"] = json!("hidden");
    assert_eq!(
        validate_response(&request, &response),
        Err(ValidationError::Schema)
    );
    Ok(())
}

#[test]
fn scopes_identity_accounting_and_availability_fail_closed() -> Result<(), ValidationError> {
    let request = request();
    for scope in ["research", "profile", "player"] {
        let mut invalid = request.clone();
        invalid["query"]["binding"]["visibility_scope"] = json!(scope);
        assert_eq!(validate_request(&invalid), Err(ValidationError::Scope));
    }
    let valid = response(&request)?;
    let mut invalid = valid.clone();
    invalid["correlation_id"] = json!("another-query");
    assert_eq!(
        validate_response(&request, &invalid),
        Err(ValidationError::Identity)
    );
    invalid = valid.clone();
    invalid["result"]["page"]["accounting"]["text_bytes"] = json!(0);
    assert_eq!(
        validate_response(&request, &invalid),
        Err(ValidationError::Accounting)
    );
    invalid = valid;
    invalid["result"]["page"]["items"][0]["fields"][0]["availability"] = json!("redacted");
    assert_eq!(
        validate_response(&request, &invalid),
        Err(ValidationError::Schema)
    );
    Ok(())
}

#[test]
fn duplicate_keys_trailing_values_and_oversize_are_rejected() {
    assert_eq!(
        decode_strict(br#"{"query":{"scope":1,"scope":2}}"#),
        Err(ValidationError::Json)
    );
    assert_eq!(decode_strict(b"{}{}"), Err(ValidationError::Json));
    assert_eq!(
        decode_strict(&vec![b' '; MAX_MESSAGE_BYTES + 1]),
        Err(ValidationError::Bounds)
    );
}

#[test]
fn capabilities_require_closed_policy_and_exact_digest() {
    let mut value = request();
    value["kind"] = json!("capabilities_response");
    value["capabilities"] = json!({
        "profile":"game-information-query-v1","query_kinds":["list"],"entity_kinds":["card"],
        "projections":["summary"],"detail_levels":["summary"],"fields":["display_name"],
        "limits":value["query"]["limits"],"max_message_bytes":262144,"max_cursor_bytes":512,
        "snapshot_policy":{"supports_live":true,"lifetime_generations":1,"max_retained_snapshots":1,
            "expiry_behavior":"reject_stale_snapshot",
            "invalidated_by":["restore","restart","profile_change","content_change","run_change","epoch_change"]}
    });
    value["query"] = Value::Null;
    assert_eq!(validate_capabilities(&value), Ok(()));
    value["capabilities"]["snapshot_policy"]["seed"] = json!(1);
    assert_eq!(validate_capabilities(&value), Err(ValidationError::Schema));
    if let Some(object) = value["capabilities"]["snapshot_policy"].as_object_mut() {
        object.remove("seed");
    }
    value["schema_digest"] = json!("0".repeat(64));
    assert_eq!(validate_capabilities(&value), Err(ValidationError::Schema));
}

#[test]
fn live_parent_fence_and_static_instance_filters_are_checked() {
    let mut query = request();
    let instance = json!({"instance_id":"synthetic-instance","run_id":"synthetic-run","epoch":2,"entity_kind":"card","entity_id":"synthetic-occurrence"});
    let snapshot =
        json!({"snapshot_id":"synthetic-snapshot","instance_ref":instance,"state_generation":7});
    query["query"]["binding"]["mode"] = json!("live");
    query["query"]["binding"]["visibility_scope"] = json!("player");
    query["query"]["binding"]["instance_ref"] = instance.clone();
    query["query"]["binding"]["snapshot_ref"] = snapshot.clone();
    query["query"]["parent_observation"] =
        json!({"instance_ref":instance,"snapshot_ref":snapshot,"state_generation":7});
    assert_eq!(validate_request(&query), Ok(()));
    query["query"]["parent_observation"]["state_generation"] = json!(8);
    assert_eq!(validate_request(&query), Err(ValidationError::Identity));
    let mut query = request();
    query["query"]["filters"]["instance_ids"] = json!(["synthetic-occurrence"]);
    assert_eq!(validate_request(&query), Err(ValidationError::Scope));
}

#[test]
fn ordering_duplicates_missing_fields_and_utf8_accounting_are_checked()
-> Result<(), ValidationError> {
    let request = request();
    let valid = response(&request)?;
    let mut invalid = valid.clone();
    let item = invalid["result"]["page"]["items"][0].clone();
    invalid["result"]["page"]["items"] = json!([item, item]);
    assert_eq!(
        validate_response(&request, &invalid),
        Err(ValidationError::Identity)
    );
    invalid = valid.clone();
    invalid["result"]["page"]["items"][0]["fields"] = json!([]);
    assert_eq!(
        validate_response(&request, &invalid),
        Err(ValidationError::Fields)
    );
    invalid = valid.clone();
    invalid["result"]["page"]["items"][0]["fields"][0]["value"] = json!("é");
    assert_eq!(
        validate_response(&request, &invalid),
        Err(ValidationError::Accounting)
    );
    invalid = valid;
    invalid["result"]["page"]["ordering"]["algorithm"] = json!("unicode_scalar_values");
    assert_eq!(
        validate_response(&request, &invalid),
        Err(ValidationError::Ordering)
    );
    Ok(())
}

fn page_with(
    request: &Value,
    coverage: &str,
    total_count_known: bool,
    total_count: Value,
    items: Value,
    text_bytes: u64,
) -> Result<Value, ValidationError> {
    let mut value = response(request)?;
    value["result"]["page"]["items"] = items;
    value["result"]["page"]["coverage"] = json!(coverage);
    value["result"]["page"]["total_count_known"] = json!(total_count_known);
    value["result"]["page"]["total_count"] = total_count;
    value["result"]["page"]["final_page"] = json!(true);
    if let Some(object) = value["result"]["page"].as_object_mut() {
        object.remove("accounting");
    }
    let items = array(&value["result"]["page"]["items"])?.to_vec();
    let item_bytes = items
        .iter()
        .map(encoded_len)
        .try_fold(0usize, |max, bytes| {
            Ok::<usize, ValidationError>(max.max(bytes?))
        })?;
    let payload_bytes = encoded_len(&value["result"]["page"]["items"])?;
    let page_bytes = encoded_len(&value["result"]["page"])?;
    value["result"]["page"]["accounting"] = json!({
        "item_count":items.len(),"item_bytes":item_bytes,"payload_bytes":payload_bytes,
        "page_bytes":page_bytes,"text_bytes":text_bytes
    });
    Ok(value)
}

#[test]
fn unavailable_and_not_observable_coverage_require_unknown_total() -> Result<(), ValidationError> {
    // Coverage extremes must never carry a known or fabricated total: downstream consumers
    // must not be able to turn "unknown" into zero or an invented count.
    let request = request();
    for coverage in ["unavailable", "not_observable"] {
        assert_eq!(
            validate_response(
                &request,
                &page_with(&request, coverage, false, Value::Null, json!([]), 0)?
            ),
            Ok(()),
            "an unknown total must be accepted for {coverage}"
        );
        for total in [json!(0), json!(42)] {
            assert_eq!(
                validate_response(
                    &request,
                    &page_with(&request, coverage, true, total.clone(), json!([]), 0)?
                ),
                Err(ValidationError::Accounting),
                "{coverage} must not carry a known total (total={total})"
            );
        }
        // A numeric total alongside `total_count_known: false` is already rejected by the
        // pinned schema layer; the requirement is that it never validates.
        assert!(
            validate_response(
                &request,
                &page_with(&request, coverage, false, json!(42), json!([]), 0)?
            )
            .is_err(),
            "{coverage} must not validate a numeric total while unknown"
        );
    }
    let item = response(&request)?["result"]["page"]["items"].clone();
    assert_eq!(
        validate_response(
            &request,
            &page_with(&request, "unavailable", false, Value::Null, item, 5)?
        ),
        Err(ValidationError::Accounting),
        "unavailable coverage must not carry items"
    );
    Ok(())
}
