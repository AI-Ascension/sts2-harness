// SPDX-License-Identifier: MIT

use super::super::{INSTANCE, LEASE, LEASE_EPOCH, LOCALE, MANIFEST, SESSION};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

static OBSERVATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(super) fn downstream_response(
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    request: &Value,
    mismatch_manifest: bool,
) -> Result<(u16, Value), String> {
    let correlation = headers
        .get("x-sts2-correlation-id")
        .map(String::as_str)
        .unwrap_or_default();
    match (method, path) {
        ("POST", "/api/v1/game-information/lookup-binding") => Ok((
            200,
            lookup_binding_response(request, correlation, mismatch_manifest)?,
        )),
        ("GET", "/api/v1/game-information/capabilities") => {
            Ok((200, game_information_capabilities(correlation)))
        }
        ("POST", "/api/v1/game-information/list") => {
            Ok((200, game_information_page(request, correlation)?))
        }
        ("POST", "/api/v1/game-information/live-observation-bootstrap") => Ok((
            200,
            live_observation_bootstrap(request, correlation, mismatch_manifest)?,
        )),
        ("GET", "/api/v3/runtime/state") => {
            Ok((200, gameplay_state("state_response", correlation)))
        }
        ("GET", "/api/v3/runtime/legal-actions") => {
            Ok((200, gameplay_state("legal_actions_response", correlation)))
        }
        ("GET", "/api/v3/runtime/reobserve") => {
            Ok((200, gameplay_state("reobserve_response", correlation)))
        }
        ("POST", "/api/v3/runtime/action") => Ok((200, gameplay_dispatch(request, correlation)?)),
        ("POST", "/api/v3/runtime/wait") => Ok((200, gameplay_wait(request, correlation)?)),
        _ => Err(format!("unexpected synthetic mod route {method} {path}")),
    }
}

fn lookup_binding_response(
    request: &Value,
    correlation: &str,
    mismatch_manifest: bool,
) -> Result<Value, String> {
    let operation = request["operation"]
        .as_str()
        .ok_or("lookup-binding operation missing")?;
    let mut response: Value = serde_json::from_str(if operation == "discovery" {
        include_str!("../../../../../protocol-artifact/game-information-lookup-binding-v1/golden/discovery-response.json")
    } else {
        include_str!("../../../../../protocol-artifact/game-information-lookup-binding-v1/golden/observation-response.json")
    })
    .map_err(|_| String::from("lookup-binding golden could not be read"))?;
    let manifest = if mismatch_manifest {
        "foreign-content"
    } else {
        MANIFEST
    };
    let scope = json!({
        "project_id":request["project_id"],
        "run_id":request["run_id"],
        "episode_id":request["episode_id"],
        "agent_id":request["agent_id"]
    });
    response["correlation_id"] = json!(correlation);
    response["binding"]["scope"] = scope;
    response["binding"]["instance_id"] = json!(INSTANCE);
    response["binding"]["authority_epoch"] = request["authority_epoch"].clone();
    response["binding"]["content_manifest_id"] = json!(manifest);
    response["binding"]["game_profile"] = json!("sts2-native-v1");
    response["binding"]["locale"] = json!(LOCALE);
    let id = binding_id(
        &request["project_id"],
        &request["run_id"],
        &request["episode_id"],
        &request["agent_id"],
        &request["authority_epoch"],
        manifest,
    )?;
    response["binding"]["binding_id"] = json!(id);
    if operation == "observe" {
        response["observation"]["binding_id"] = json!(id);
        // The session rejects a repeated observation identity as a stale snapshot.
        // Keep this producer witness distinct on every observe call, including
        // retries whose transport correlation may be reused by a peer.
        let sequence = OBSERVATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        response["observation"]["observation_id"] = json!(format!("peer-observation-{sequence}"));
        response["observation"]["snapshot_id"] = json!("peer-snapshot-1");
        response["observation"]["state_generation"] = json!(0);
    }
    Ok(response)
}

fn binding_id(
    project: &Value,
    run: &Value,
    episode: &Value,
    agent: &Value,
    epoch: &Value,
    manifest: &str,
) -> Result<String, String> {
    let identity = BTreeMap::from([
        (String::from("agent_id"), agent.clone()),
        (String::from("authority_epoch"), epoch.clone()),
        (String::from("content_manifest_id"), json!(manifest)),
        (String::from("episode_id"), episode.clone()),
        (String::from("game_profile"), json!("sts2-native-v1")),
        (String::from("locale"), json!(LOCALE)),
        (String::from("project_id"), project.clone()),
        (String::from("run_id"), run.clone()),
    ]);
    serde_json::to_vec(&identity)
        .map(sts2_harness::sha256_hex)
        .map_err(|_| String::from("lookup-binding identity could not be encoded"))
}

fn game_information_capabilities(correlation: &str) -> Value {
    json!({
        "protocol_version":"game-information-query-v1",
        "schema_digest":"376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9",
        "provenance":{"artifact":"sts2-protocol/game-information-query-v1",
            "source":"schemas/game-information-query-v1.schema.json","generator":"hand-authored"},
        "correlation_id":correlation,"kind":"capabilities_response",
        "query":null,"result":null,"error":null,
        "capabilities":{
            "profile":"game-information-query-v1",
            "query_kinds":["list"],"entity_kinds":["card"],
            "projections":["summary"],"detail_levels":["summary"],"fields":["display_name"],
            "limits":{"page_items":4,"item_bytes":4096,"page_bytes":65536,"text_bytes":4096},
            "max_message_bytes":262144,"max_cursor_bytes":512,
            "snapshot_policy":{
                "supports_live":true,"lifetime_generations":128,"max_retained_snapshots":8,
                "expiry_behavior":"reject_stale_snapshot",
                "invalidated_by":["content_change","epoch_change","profile_change","restore","restart","run_change"]
            }
        }
    })
}

fn game_information_page(request: &Value, correlation: &str) -> Result<Value, String> {
    let query = request
        .get("query")
        .cloned()
        .ok_or("game-information query missing")?;
    let mut page = json!({
        "items":[],"final_page":true,"next_cursor":null,"cursor_binding":null,
        "coverage":"complete","total_count_known":true,"total_count":0,
        "ordering":{"key":"definition_ref","direction":"ascending",
            "algorithm":"identity_bytes","deterministic":true},
        "limits":query["limits"]
    });
    let page_without_accounting = serde_json::to_vec(&page)
        .map_err(|_| String::from("game-information page could not be encoded"))?;
    page["accounting"] = json!({
        "item_count":0,"item_bytes":0,"payload_bytes":2,
        "page_bytes":page_without_accounting.len(),"text_bytes":0
    });
    Ok(json!({
        "protocol_version":"game-information-query-v1",
        "schema_digest":"376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9",
        "provenance":{"artifact":"sts2-protocol/game-information-query-v1",
            "source":"schemas/game-information-query-v1.schema.json","generator":"hand-authored"},
        "correlation_id":correlation,"kind":"query_response",
        "query":query,
        "result":{"read_only":true,"parent_observation":null,"result_generation":null,"page":page},
        "capabilities":null,"error":null
    }))
}

fn live_observation_bootstrap(
    request: &Value,
    correlation: &str,
    mismatch_manifest: bool,
) -> Result<Value, String> {
    let manifest = if mismatch_manifest {
        "foreign-content"
    } else {
        MANIFEST
    };
    let definition = request
        .get("selector")
        .and_then(|selector| selector.get("definition_ref"))
        .cloned()
        .unwrap_or_else(|| {
            json!({"content_manifest_id":manifest,"entity_kind":"card",
                "namespaced_id":"ironclad:strike","variant":null})
        });
    let instance = json!({
        "instance_id": INSTANCE,
        "run_id": request["scope"]["run_id"],
        "epoch": 7,
        "entity_kind": "card",
        "entity_id": "card-17"
    });
    let snapshot = json!({
        "snapshot_id": "peer-snapshot-1",
        "instance_ref": instance,
        "state_generation": 0
    });
    Ok(json!({
        "protocol_version":"game-information-live-observation-bootstrap-v1",
        "schema_digest":"6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2",
        "provenance":{"artifact":"sts2-protocol/game-information-live-observation-bootstrap-v1",
            "source":"schemas/game-information-live-observation-bootstrap-v1.schema.json","generator":"hand-authored"},
        "correlation_id":correlation,"kind":"bootstrap_response",
        "scope":{"instance_id":INSTANCE,"run_id":request["scope"]["run_id"],
            "authority_epoch":request["scope"]["authority_epoch"],
            "content_manifest_id":manifest,"locale":LOCALE},
        "selector":{"definition_ref":definition,"instance_ref":null},
        "limits":{"max_visible_entities":64,"max_item_bytes":65536,"max_message_bytes":262144},
        "parent_observation":{"instance_ref":instance,"snapshot_ref":snapshot,"state_generation":0},
        "visible_entities":[{"definition_ref":definition,"instance_ref":instance,"snapshot_ref":snapshot}],
        "owner_provenance":{"native_snapshot_owner":"sts2-game-mod",
            "content_manifest_owner":"sts2-game-mod","instance_fence_owner":"sts2-gateway",
            "authority_epoch_owner":"sts2-harness","instance_ref_epoch_owner":"sts2-game-mod",
            "transport_lease_epoch_role":"fence_only"},
        "error":null
    }))
}

fn gameplay_state(kind: &str, correlation: &str) -> Value {
    let mut response: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))
    .expect("Runtime-v3 state golden is valid");
    response["correlation_id"] = json!(correlation);
    response["instance_id"] = json!(INSTANCE);
    response["session_id"] = json!(SESSION);
    response["lease_id"] = json!(LEASE);
    response["lease_epoch"] = json!(LEASE_EPOCH);
    response["kind"] = json!(kind);
    response["generation"] = json!(0);
    response["state_id"] = json!("combat-1");
    response["observation"]["state_id"] = json!("combat-1");
    response["observation"]["generation"] = json!(0);
    response["observation"]["state"] = json!({"state":"combat","turn_index":1,"enemies":[]});
    response["legal_actions"] = json!([
        {"action_id":"combat.end-turn","action":{"kind":"end_turn"}}
    ]);
    if kind == "legal_actions_response" {
        response["observation"] = Value::Null;
    }
    response
}

fn gameplay_dispatch(request: &Value, correlation: &str) -> Result<Value, String> {
    let mut response: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))
    .map_err(|_| String::from("Runtime-v3 settled golden could not be read"))?;
    response["correlation_id"] = json!(correlation);
    response["instance_id"] = json!(request["instance_id"]);
    response["session_id"] = json!(request["session_id"]);
    response["lease_id"] = json!(request["lease_id"]);
    response["lease_epoch"] = request["lease_epoch"].clone();
    response["operation_id"] = request["operation_id"].clone();
    response["state_id"] = request["state_id"].clone();
    response["generation"] = json!(1);
    // Runtime-v3 response envelopes keep the submitted action on the request
    // side; a settled response must leave this field null per the schema.
    response["action"] = Value::Null;
    response["observation"]["state_id"] = request["state_id"].clone();
    response["observation"]["generation"] = json!(1);
    response["observation"]["state"] = json!({"state":"victory"});
    response["transition"]["state_id"] = request["state_id"].clone();
    let from_generation = request["generation"]
        .as_u64()
        .unwrap_or(1)
        .saturating_sub(1);
    response["transition"]["from_generation"] = json!(from_generation);
    response["transition"]["to_generation"] = json!(1);
    Ok(response)
}

fn gameplay_wait(request: &Value, correlation: &str) -> Result<Value, String> {
    let mut response: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))
    .map_err(|_| String::from("Runtime-v3 wait golden could not be read"))?;
    response["correlation_id"] = json!(correlation);
    response["instance_id"] = json!(request["instance_id"]);
    response["session_id"] = json!(request["session_id"]);
    response["lease_id"] = json!(request["lease_id"]);
    response["lease_epoch"] = request["lease_epoch"].clone();
    response["operation_id"] = request["operation_id"].clone();
    response["state_id"] = json!("combat-1");
    response["generation"] = json!(1);
    response["kind"] = json!("wait_response");
    response["status"] = json!("settled");
    response["wait_for_millis"] = Value::Null;
    response["wait_outcome"] = json!("successor");
    response["action"] = Value::Null;
    response["observation"]["state_id"] = json!("combat-1");
    response["observation"]["generation"] = json!(1);
    response["observation"]["state"] = json!({"state":"victory"});
    let from_generation = request["generation"]
        .as_u64()
        .unwrap_or(1)
        .saturating_sub(1);
    response["transition"]["from_generation"] = json!(from_generation);
    response["transition"]["to_generation"] = json!(1);
    response["transition"]["state_id"] = json!("combat-1");
    response["transition"]["effect_kind"] = json!("end_turn.settled");
    Ok(response)
}
