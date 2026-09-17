// SPDX-License-Identifier: MIT
fn live_bootstrap(
    port: &mut RuntimeV3Port,
    request: &serde_json::Value,
) -> Result<Vec<u8>, LookupError> {
    let lbr = port.lookup_binding.as_ref().ok_or(LookupError::Scope)?;
    let discovered = lbr.binding().ok_or(LookupError::Scope)?;
    let observation = lbr.observation().ok_or(LookupError::Reobserve)?;
    if observation.snapshot_id.is_empty() {
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
    let context = LookupMcpContext {
        instance_id: port.config.instance_id.clone(),
        mcp_session_id: port.config.mcp_session_id.clone(),
        lease_id: port.config.lease_id.clone(),
        lease_epoch: port.config.lease_epoch,
    };
    let call = || {
        sts2_harness::game_information::call_live_observation_bootstrap_mcp(
            &context,
            &body,
            |id, arguments| {
                wire::rpc_call_catalog_read(
                    port.mcp.as_mut().ok_or(LookupError::Transport)?,
                    id,
                    "tools/call",
                    arguments,
                )
                .map_err(|_| LookupError::Transport)
            },
        )
    };
    let response = owner.call_with_lookup_revalidation(&expected, call)?;
    let value: serde_json::Value =
        serde_json::from_slice(&response).map_err(|_| LookupError::Invalid)?;
    let selected = sts2_harness::game_information_binding::game_information_bootstrap::select_snapshot(&body, &value).map_err(|error| {
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
    let parent = value["parent_observation"]
        .as_object()
        .ok_or(LookupError::Invalid)?;
    let parent_instance = parent["instance_ref"]
        .as_object()
        .ok_or(LookupError::Invalid)?;
    let parent_snapshot = parent["snapshot_ref"]
        .as_object()
        .ok_or(LookupError::Invalid)?;
    if parent_instance["instance_id"] != json!(port.config.instance_id)
        || parent_instance["run_id"] != json!(port.config.run_id)
        || parent_snapshot["snapshot_id"] != json!(observation.snapshot_id)
        || parent["state_generation"] != json!(observation.state_generation)
        || selected["state_generation"] != json!(observation.state_generation)
    {
        return Err(LookupError::Reobserve);
    }
    Ok(response)
}
