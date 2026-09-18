// SPDX-License-Identifier: MIT

use super::super::{BINDING_STATE_GENERATION, INSTANCE, LEASE_EPOCH, LOCALE, PeerNegative};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "runtime_v3_game_information_entry_live_peer_gameplay.rs"]
mod gameplay;
use gameplay::{gameplay_dispatch, gameplay_state, gameplay_wait};

static OBSERVATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(super) fn downstream_response(
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    request: &Value,
    negative: PeerNegative,
) -> Result<(u16, Value), String> {
    let correlation = headers
        .get("x-sts2-correlation-id")
        .map(String::as_str)
        .unwrap_or_default();
    match (method, path) {
        ("POST", "/api/v1/game-information/lookup-binding") => Ok((
            200,
            lookup_binding_response(request, correlation, negative)?,
        )),
        ("GET", "/api/v1/game-information/capabilities") => {
            Ok((200, game_information_capabilities(correlation)))
        }
        ("POST", "/api/v1/game-information/list") => {
            Ok((200, game_information_page(request, correlation)?))
        }
        ("POST", "/api/v1/game-information/detail") => {
            Ok((200, game_information_page(request, correlation)?))
        }
        ("POST", "/api/v1/game-information/live-observation-bootstrap") => {
            live_observation_bootstrap(request, correlation, negative)
        }
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
    negative: PeerNegative,
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
    let manifest = negative.content_manifest_id();
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
        response["observation"]["state_generation"] = json!(BINDING_STATE_GENERATION);
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
            "query_kinds":["list","detail"],"entity_kinds":["card"],
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
    // A live result must echo the query's parent observation and answer at the
    // snapshot generation the owner bound; a static result carries neither.
    let (parent, generation) = if query["binding"]["mode"] == "live" {
        (
            query["parent_observation"].clone(),
            query["binding"]["snapshot_ref"]["state_generation"].clone(),
        )
    } else {
        (Value::Null, Value::Null)
    };
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
        "result":{"read_only":true,"parent_observation":parent,"result_generation":generation,"page":page},
        "capabilities":null,"error":null
    }))
}

fn live_observation_bootstrap(
    request: &Value,
    correlation: &str,
    negative: PeerNegative,
) -> Result<(u16, Value), String> {
    if negative == PeerNegative::NotObservable {
        return Ok((503, not_observable_bootstrap(request, correlation)?));
    }
    let manifest = negative.content_manifest_id();
    let generation = negative.bootstrap_state_generation();
    let definition = request
        .get("selector")
        .and_then(|selector| selector.get("definition_ref"))
        .or_else(|| request.get("definition_ref"))
        .cloned()
        .unwrap_or_else(|| {
            json!({"content_manifest_id":manifest,"entity_kind":"card",
                "namespaced_id":"ironclad:strike","variant":null})
        });
    let scope = request.get("scope").cloned().unwrap_or_else(|| {
        json!({
            "instance_id": request["instance_id"],
            "run_id": request["run_id"],
            "authority_epoch": request["authority_epoch"],
            "content_manifest_id": request["content_manifest_id"],
            "locale": request["locale"]
        })
    });
    let instance = json!({
        "instance_id": INSTANCE,
        "run_id": scope["run_id"],
        // The Gateway fences a live instance reference to the transport lease epoch.
        "epoch": LEASE_EPOCH,
        "entity_kind": "card",
        "entity_id": "card-17"
    });
    let snapshot = json!({
        "snapshot_id": "peer-snapshot-1",
        "instance_ref": instance,
        "state_generation": generation
    });
    Ok((
        200,
        json!({
            "protocol_version":"game-information-live-observation-bootstrap-v1",
            "schema_digest":"6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2",
            "provenance":{"artifact":"sts2-protocol/game-information-live-observation-bootstrap-v1",
                "source":"schemas/game-information-live-observation-bootstrap-v1.schema.json","generator":"hand-authored"},
            "correlation_id":correlation,"kind":"bootstrap_response",
            "scope":{"instance_id":INSTANCE,"run_id":scope["run_id"],
                "authority_epoch":scope["authority_epoch"],
                "content_manifest_id":manifest,"locale":LOCALE},
            "selector":{"definition_ref":definition,"instance_ref":null},
            "limits":request.get("limits").cloned().unwrap_or_else(|| json!({
                "max_visible_entities":64,"max_item_bytes":65536,"max_message_bytes":262144})),
            "parent_observation":{"instance_ref":instance,"snapshot_ref":snapshot,"state_generation":generation},
            "visible_entities":[{"definition_ref":definition,"instance_ref":instance,"snapshot_ref":snapshot}],
            "owner_provenance":{"native_snapshot_owner":"sts2-game-mod",
                "content_manifest_owner":"sts2-game-mod","instance_fence_owner":"sts2-gateway",
                "authority_epoch_owner":"sts2-harness","instance_ref_epoch_owner":"sts2-game-mod",
                "transport_lease_epoch_role":"fence_only"},
            "error":null
        }),
    ))
}

/// The shipped protocol golden is the producer's `not_observable` shape. The pinned
/// Gateway and MCP both require an error response to echo the exact request scope
/// and selector under a 4xx/5xx status, so those and the transport correlation are
/// the only substitutions.
fn not_observable_bootstrap(request: &Value, correlation: &str) -> Result<Value, String> {
    let mut response: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/game-information-live-observation-bootstrap-v1/golden/error-native-unavailable.json"
    ))
    .map_err(|_| String::from("bootstrap not-observable golden could not be read"))?;
    response["correlation_id"] = json!(correlation);
    response["scope"] = request
        .get("scope")
        .cloned()
        .ok_or("bootstrap request scope missing")?;
    response["selector"] = request
        .get("selector")
        .cloned()
        .ok_or("bootstrap request selector missing")?;
    Ok(response)
}
