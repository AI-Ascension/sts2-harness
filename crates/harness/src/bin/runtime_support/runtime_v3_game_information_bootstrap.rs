// SPDX-License-Identifier: MIT
fn live_bootstrap(
    port: &mut RuntimeV3Port,
    request: &serde_json::Value,
) -> Result<Vec<u8>, LookupError> {
    let lbr = port.lookup_binding.as_ref().ok_or(LookupError::Scope)?;
    let discovered = lbr.binding().ok_or(LookupError::Scope)?;
    if lbr.observation().is_none() {
        return Err(LookupError::Reobserve);
    }
    let owner = port
        .lookup_policy_owner
        .clone()
        .ok_or(LookupError::Scope)?;
    let expected = port
        .lookup_policy_binding
        .clone()
        .ok_or(LookupError::Scope)?;
    owner
        .lookup_snapshot(Some(&expected))
        .map_err(|_| LookupError::Scope)?;
    let mut body = request.clone();
    body["scope"] = json!({
        "instance_id": port.config.instance_id, "run_id": port.config.run_id,
        "authority_epoch": expected.fence.owner_epoch,
        "content_manifest_id": discovered.content_manifest_id, "locale": discovered.locale
    });
    body["correlation_id"] = json!(port.next_rpc_id.to_string());
    let correlation = body["correlation_id"]
        .as_str()
        .ok_or(LookupError::Invalid)?
        .to_owned();
    sts2_harness::game_information_binding::game_information_bootstrap::validate_request(&body)
        .map_err(|error| match error {
        sts2_harness::game_information_binding::game_information_bootstrap::BootstrapError::Bounds => LookupError::Bounds,
        sts2_harness::game_information_binding::game_information_bootstrap::BootstrapError::Scope => LookupError::Scope,
        _ => LookupError::Invalid,
    })?;
    if body["selector"]["definition_ref"]["content_manifest_id"]
        != json!(discovered.content_manifest_id)
    {
        return Err(LookupError::Scope);
    }
    port.next_rpc_id = port.next_rpc_id.checked_add(1).ok_or(LookupError::Bounds)?;
    let path = format!(
        "/v1/instances/{}/game-information/live-observation-bootstrap",
        port.config.instance_id
    );
    let call = || {
        port.gateway
            .request_bytes(
                "POST",
                &path,
                &body,
                super::identity_headers(&port.config, &correlation),
            )
            .map_err(|_| LookupError::Transport)
    };
    let response = owner
        .call_with_lookup_revalidation(&expected, call)
        .map_err(|error| {
            if error == LookupError::Scope {
                LookupError::Scope
            } else {
                LookupError::Transport
            }
        })?;
    let value: serde_json::Value =
        serde_json::from_slice(&response).map_err(|_| LookupError::Invalid)?;
    sts2_harness::game_information_binding::game_information_bootstrap::select_snapshot(&body, &value).map_err(|error| {
        match error {
            sts2_harness::game_information_binding::game_information_bootstrap::BootstrapError::Bounds => LookupError::Bounds,
            sts2_harness::game_information_binding::game_information_bootstrap::BootstrapError::Scope => LookupError::Scope,
            sts2_harness::game_information_binding::game_information_bootstrap::BootstrapError::Ambiguous
            | sts2_harness::game_information_binding::game_information_bootstrap::BootstrapError::Unavailable => {
                LookupError::MissingCapability
            }
            sts2_harness::game_information_binding::game_information_bootstrap::BootstrapError::Invalid => LookupError::Invalid,
        }
    })?;
    Ok(response)
}
