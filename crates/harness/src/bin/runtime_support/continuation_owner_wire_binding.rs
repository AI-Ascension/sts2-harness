// SPDX-License-Identifier: MIT

pub(super) fn validate_allocation_binding(
    owner: &Value,
    config: &RuntimeConfig,
    authority: Option<&RecoveryAuthority>,
) -> Result<(), String> {
    let authority = authority.ok_or_else(|| {
        String::from("selected branch owner claim requires an allocated recovery authority")
    })?;
    let fence = &authority.current_fence;
    for (field, expected) in [
        ("deployment_id", authority.deployment_id.as_str()),
        ("instance_id", config.instance_id.as_str()),
        (
            "instance_incarnation",
            authority.instance_incarnation.as_str(),
        ),
        ("boot_id", authority.boot_id.as_str()),
        (
            "host_fence_id",
            fence["host_fence_id"].as_str().unwrap_or_default(),
        ),
        ("lease_id", config.lease_id.as_str()),
        ("session_id", config.session_id.as_str()),
    ] {
        if owner[field].as_str() != Some(expected) {
            return Err(format!(
                "gateway current owner {field} does not match the allocated runtime"
            ));
        }
    }
    if owner["authority_generation"].as_u64() != Some(authority.authority_generation)
        || owner["host_fence_generation"].as_u64() != fence["fence_generation"].as_u64()
        || owner["lease_epoch"].as_u64() != Some(config.lease_epoch)
    {
        return Err(String::from(
            "gateway current owner fence generation does not match the runtime allocation",
        ));
    }
    Ok(())
}

pub(super) fn ensure_lookup_matches(
    response: &Value,
    operation_id: &str,
    expected_owner: &Value,
) -> Result<(), String> {
    if response["kind"] != "owner_claim_lookup_response" {
        return Err(String::from(
            "gateway owner lookup response kind is invalid",
        ));
    }
    ensure_current_owner(response, expected_owner)?;
    if response["payload"]["claim_state"] != "historical"
        || response["payload"]["claim"]["operation_id"] != operation_id
        || response["payload"]["claim"]["owner"] != *expected_owner
        || !is_lower_sha256(response["payload"]["claim"]["request_digest"].as_str())
    {
        return Err(String::from(
            "gateway has no matching historical claim for the selected branch operation",
        ));
    }
    Ok(())
}

pub(super) fn ensure_current_owner(response: &Value, expected_owner: &Value) -> Result<(), String> {
    let current = &response["payload"]["current_owner"];
    if current["state"] != "available" || current["owner"] != *expected_owner {
        return Err(String::from(
            "gateway current owner no longer matches the persisted selected-branch fence",
        ));
    }
    Ok(())
}
