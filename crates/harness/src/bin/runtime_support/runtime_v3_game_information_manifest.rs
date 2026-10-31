// SPDX-License-Identifier: MIT
// Read through the existing MCP owner and gateway negotiation, with owner revalidation.

fn content_manifest(port: &mut RuntimeV3Port) -> Result<(String, Vec<u8>), LookupError> {
    let owner = port.lookup_policy_owner.clone();
    let expected = port.lookup_policy_binding.clone();
    let required = port.lookup_binding_required;
    let call = || manifest_exchange(port);
    if required {
        owner.as_ref().ok_or(LookupError::Scope)?.call_with_lookup_revalidation(
            expected.as_ref().ok_or(LookupError::Scope)?, call,
        )
    } else {
        Err(LookupError::Scope)
    }
}

fn manifest_exchange(port: &mut RuntimeV3Port) -> Result<(String, Vec<u8>), LookupError> {
    let catalog_id = port.next_rpc_id;
    let id = catalog_id.checked_add(1).ok_or(LookupError::Bounds)?;
    port.next_rpc_id = id.checked_add(1).ok_or(LookupError::Bounds)?;
    let mcp = port.mcp.as_mut().ok_or(LookupError::Transport)?;
    let catalog = wire::rpc_call_catalog_read(mcp, catalog_id, "tools/list", json!({}))
        .map_err(|_| LookupError::Transport)?;
    wire::validate_catalog(&catalog, "negotiated-composition-v1")
        .map_err(|_| LookupError::MissingCapability)?;
    if !catalog["result"]["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool| {
        tool["name"] == "sts2.game_information_content_manifest"
    })) {
        return Err(LookupError::MissingCapability);
    }
    let context = LookupMcpContext {
        instance_id: port.config.instance_id.clone(), mcp_session_id: port.config.mcp_session_id.clone(),
        lease_id: port.config.lease_id.clone(), lease_epoch: port.config.lease_epoch,
    };
    // This read has its own typed protocol refusal; only this fixed dispatch admits that arm.
    let bytes = call_content_manifest_mcp(&context, id, |id, arguments| {
        wire::rpc_call_manifest_read(mcp, id, arguments).map_err(|_| LookupError::Transport)
    })?;
    Ok((id.to_string(), bytes))
}
