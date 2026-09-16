// SPDX-License-Identifier: MIT

fn validate_response_payload(frame: &Value, expected_kind: &str) -> Result<(), String> {
    match expected_kind {
        "current_owner_response" => {
            let payload = &frame["payload"];
            exact_object(payload, &["state", "owner"], "current-owner payload")?;
            validate_current_owner_shape(payload)
        }
        "owner_claim_response" => {
            let payload = &frame["payload"];
            exact_object(payload, &["result", "claim"], "owner-claim payload")?;
            let claim = exact_object(
                &payload["claim"],
                &[
                    "operation_id",
                    "request_digest",
                    "owner",
                    "claimed_at_millis",
                ],
                "owner-claim record",
            )?;
            if claim.len() != 4
                || !valid_uuid_v4(payload["claim"]["operation_id"].as_str())
                || !is_lower_sha256(payload["claim"]["request_digest"].as_str())
                || payload["claim"]["claimed_at_millis"]
                    .as_u64()
                    .is_none_or(|value| value > MAX_SAFE_INTEGER)
            {
                return Err(String::from(
                    "gateway continuation-owner claim payload is invalid",
                ));
            }
            validate_owner_shape(&payload["claim"]["owner"])
        }
        "owner_claim_lookup_response" => {
            let payload = &frame["payload"];
            exact_object(
                payload,
                &["claim_state", "claim", "current_owner"],
                "owner-lookup payload",
            )?;
            validate_current_owner_shape(&payload["current_owner"])?;
            match payload["claim_state"].as_str() {
                Some("not_found") if payload["claim"].is_null() => Ok(()),
                Some("historical") => validate_claim_record(&payload["claim"]),
                _ => Err(String::from(
                    "gateway continuation-owner lookup payload is invalid",
                )),
            }
        }
        _ => Err(String::from(
            "unsupported continuation-owner response payload",
        )),
    }
}

fn validate_current_owner_shape(payload: &Value) -> Result<(), String> {
    let payload = exact_object(payload, &["state", "owner"], "current-owner result")?;
    match payload["state"].as_str() {
        Some("available") => validate_owner_shape(&payload["owner"]),
        Some("absent" | "expired" | "revoked") if payload["owner"].is_null() => Ok(()),
        Some("unknown") => {
            if !payload["owner"].is_null() {
                validate_owner_shape(&payload["owner"])?;
            }
            Ok(())
        }
        _ => Err(String::from(
            "gateway continuation-owner state payload is invalid",
        )),
    }
}

fn validate_claim_record(claim: &Value) -> Result<(), String> {
    exact_object(
        claim,
        &[
            "operation_id",
            "request_digest",
            "owner",
            "claimed_at_millis",
        ],
        "owner-claim record",
    )?;
    if !valid_uuid_v4(claim["operation_id"].as_str())
        || !is_lower_sha256(claim["request_digest"].as_str())
        || claim["claimed_at_millis"]
            .as_u64()
            .is_none_or(|value| value > MAX_SAFE_INTEGER)
    {
        return Err(String::from(
            "gateway continuation-owner historical claim is invalid",
        ));
    }
    validate_owner_shape(&claim["owner"])
}

fn exact_object<'a>(
    value: &'a Value,
    fields: &[&str],
    context: &str,
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("gateway continuation-owner {context} is not an object"))?;
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return Err(format!(
            "gateway continuation-owner {context} has unexpected fields"
        ));
    }
    Ok(object)
}
