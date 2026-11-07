// SPDX-License-Identifier: MIT
use serde_json::Value;
use std::collections::BTreeSet;

use super::super::runtime_v3_settings::live_admission::admitted_live_episode;

const GAMEPLAY: [&str; 6] = [
    "sts2.observe",
    "sts2.legal_actions",
    "sts2.dispatch_action",
    "sts2.wait_for_transition",
    "sts2.reobserve",
    "sts2.recover",
];
const LOOKUPS: [&str; 7] = [
    "sts2.game_information_capabilities",
    "sts2.game_information_list",
    "sts2.game_information_search",
    "sts2.game_information_get",
    "sts2.game_information_detail",
    "sts2.game_information_availability",
    "sts2.game_information_content_manifest",
];
const LOOKUP_BINDING: &str = "sts2.game_information_binding";
const LOOKUP_BOOTSTRAP: &str = "sts2.game_information.live_observation_bootstrap";

/// Admit a mixed catalog without changing any closed legacy catalog.
/// Lookup capability intersection is subsequently verified by LookupSession negotiation.
pub(super) fn validate(response: &Value) -> Result<(), String> {
    let result = &response["result"];
    if result["revision"] != "negotiated-composition-v1-mcp"
        || result["refresh_required"] != false
        || !result["session_epoch"]
            .as_u64()
            .is_some_and(|epoch| epoch <= 9_007_199_254_740_991)
        || result["composition"]["revision"] != "negotiated-composition-v1-mcp"
    {
        return Err("MCP lookup composition needs fresh negotiation".to_owned());
    }
    let tools = result["tools"]
        .as_array()
        .ok_or_else(|| "MCP lookup catalog omitted tools".to_owned())?;
    if admitted_live_episode() {
        eprintln!(
            "MCP negotiated tools: {:?}",
            tools
                .iter()
                .map(|tool| tool["name"].clone())
                .collect::<Vec<_>>()
        );
    }
    // Every admitted name is unique; include binding, bootstrap, capabilities and map.
    if tools.len() > GAMEPLAY.len() + LOOKUPS.len() + 4 {
        return Err("MCP lookup catalog exceeds supported surface".to_owned());
    }
    let mut names = BTreeSet::new();
    for tool in tools {
        let name = tool["name"]
            .as_str()
            .ok_or_else(|| "MCP lookup tool name invalid".to_owned())?;
        if !names.insert(name)
            || !(GAMEPLAY.contains(&name)
                || LOOKUPS.contains(&name)
                || name == LOOKUP_BINDING
                || name == LOOKUP_BOOTSTRAP
                || matches!(name, "sts2.capabilities" | "sts2.map_snapshot"))
        {
            return Err("MCP lookup catalog has a duplicate or unsupported tool".to_owned());
        }
        if LOOKUPS.contains(&name) {
            validate_lookup(tool)?;
        } else if name == LOOKUP_BINDING {
            validate_binding(tool)?;
        } else if name == LOOKUP_BOOTSTRAP {
            validate_bootstrap(tool)?;
        }
    }
    if GAMEPLAY.iter().any(|name| !names.contains(name)) || !names.contains("sts2.capabilities") {
        return Err("MCP lookup catalog omitted required gameplay or discovery".to_owned());
    }
    Ok(())
}

fn validate_lookup(tool: &Value) -> Result<(), String> {
    let meta = &tool["_meta"]["sts2"];
    if tool["annotations"]["readOnlyHint"] != true
        || tool["annotations"]["destructiveHint"] != false
        || tool["annotations"]["idempotentHint"] != true
        || tool["inputSchema"]["additionalProperties"] != false
        || meta["revision"] != "game-information-query-v1-mcp"
        || !matches!(
            meta["feature"].as_str(),
            Some("static_reference" | "live_details")
        )
    {
        return Err("MCP lookup descriptor has unsupported authority or revision".to_owned());
    }
    if tool["name"] == "sts2.game_information_content_manifest"
        && meta["feature"] != "static_reference"
    {
        return Err("MCP manifest descriptor has unsupported capability scope".to_owned());
    }
    Ok(())
}

fn validate_binding(tool: &Value) -> Result<(), String> {
    let meta = &tool["_meta"]["sts2"];
    if tool["annotations"]["readOnlyHint"] != true
        || tool["annotations"]["destructiveHint"] != false
        || tool["annotations"]["idempotentHint"] != true
        || tool["inputSchema"]["additionalProperties"] != false
        || meta["revision"] != "game-information-lookup-binding-v1-mcp"
        || meta["feature"] != "static_reference"
    {
        return Err(
            "MCP lookup-binding descriptor has unsupported authority or revision".to_owned(),
        );
    }
    Ok(())
}

fn validate_bootstrap(tool: &Value) -> Result<(), String> {
    let meta = &tool["_meta"]["sts2"];
    if tool["annotations"]["readOnlyHint"] != true
        || tool["annotations"]["destructiveHint"] != false
        || tool["annotations"]["idempotentHint"] != true
        || tool["inputSchema"]["additionalProperties"] != false
        || meta["revision"] != "game-information-live-observation-bootstrap-v1"
        // The MCP groups this tool under its closed `live_details` capability group;
        // no MCP capability group is named after the tool itself.
        || meta["feature"] != "live_details"
    {
        return Err("MCP bootstrap descriptor has unsupported authority or revision".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn catalog() -> Value {
        let mut tools = GAMEPLAY
            .iter()
            .map(|name| json!({"name":name}))
            .collect::<Vec<_>>();
        tools.push(json!({"name":"sts2.capabilities"}));
        tools.push(json!({"name":LOOKUPS[1],"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true},
            "inputSchema":{"additionalProperties":false},"_meta":{"sts2":{"revision":"game-information-query-v1-mcp","feature":"static_reference"}}}));
        json!({"result":{"revision":"negotiated-composition-v1-mcp","refresh_required":false,"session_epoch":1,
            "composition":{"revision":"negotiated-composition-v1-mcp"},"tools":tools}})
    }
    #[test]
    fn missing_optional_feature_is_allowed_but_stale_conflicts_and_privilege_fail() {
        let valid = catalog();
        assert!(validate(&valid).is_ok());
        let mut invalid = valid.clone();
        invalid["result"]["refresh_required"] = json!(true);
        assert!(validate(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid["result"]["tools"][7]["annotations"]["readOnlyHint"] = json!(false);
        assert!(validate(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid["result"]["tools"][7]["_meta"]["sts2"]["revision"] = json!("future-v2");
        assert!(validate(&invalid).is_err());
        let mut invalid = valid;
        invalid["result"]["tools"][7]["name"] = json!("sts2.profile_read");
        assert!(validate(&invalid).is_err());
    }

    #[test]
    fn complete_manifest_composition_keeps_every_other_admitted_tool() {
        let mut valid = catalog();
        let descriptor = valid["result"]["tools"][7].clone();
        let mut tools = GAMEPLAY
            .iter()
            .map(|name| json!({"name":name}))
            .collect::<Vec<_>>();
        for name in LOOKUPS {
            let mut tool = descriptor.clone();
            tool["name"] = json!(name);
            tools.push(tool);
        }
        for (name, revision, feature) in [
            (
                LOOKUP_BINDING,
                "game-information-lookup-binding-v1-mcp",
                "static_reference",
            ),
            (
                LOOKUP_BOOTSTRAP,
                "game-information-live-observation-bootstrap-v1",
                "live_details",
            ),
        ] {
            let mut tool = descriptor.clone();
            tool["name"] = json!(name);
            tool["_meta"]["sts2"] = json!({"revision":revision,"feature":feature});
            tools.push(tool);
        }
        tools.extend([
            json!({"name":"sts2.capabilities"}),
            json!({"name":"sts2.map_snapshot"}),
        ]);
        assert_eq!(tools.len(), 17);
        valid["result"]["tools"] = json!(tools);
        assert!(validate(&valid).is_ok());
        let mut invalid = valid.clone();
        invalid["result"]["tools"][12]["_meta"]["sts2"]["feature"] = json!("live_details");
        assert!(validate(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid["result"]["tools"][12]["annotations"]["destructiveHint"] = json!(true);
        assert!(validate(&invalid).is_err());
        let mut invalid = valid;
        invalid["result"]["tools"][12] = invalid["result"]["tools"][11].clone();
        assert!(validate(&invalid).is_err());
    }
}
