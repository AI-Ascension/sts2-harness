// SPDX-License-Identifier: MIT

use serde_json::json;

pub(super) fn mcp_server_script(log: &std::path::Path) -> String {
    let log = serde_json::to_string(&log.to_string_lossy()).expect("Python MCP log literal");
    let initial = serde_json::to_string(&state_response(
        "state_response",
        "1",
        "combat",
        41,
        json!([
            {"action_id":"combat.end-turn","action":{"kind":"end_turn"}}
        ]),
    ))
    .expect("initial MCP state");
    let initial = serde_json::to_string(&initial).expect("Python MCP initial-state literal");
    let catalog = json!({
        "revision":"negotiated-composition-v1-mcp",
        "refresh_required":false,
        "session_epoch":1,
        "composition":{"revision":"negotiated-composition-v1-mcp"},
        "tools": [
            {"name":"sts2.observe"},
            {"name":"sts2.legal_actions"},
            {"name":"sts2.dispatch_action"},
            {"name":"sts2.wait_for_transition"},
            {"name":"sts2.reobserve"},
            {"name":"sts2.recover"},
            {"name":"sts2.capabilities"},
            {"name":"sts2.game_information_capabilities","annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true},"inputSchema":{"additionalProperties":false},"_meta":{"sts2":{"revision":"game-information-query-v1-mcp","feature":"static_reference"}}},
            {"name":"sts2.game_information_list","annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true},"inputSchema":{"additionalProperties":false},"_meta":{"sts2":{"revision":"game-information-query-v1-mcp","feature":"static_reference"}}}
        ]
    });
    let catalog = serde_json::to_string(&catalog).expect("MCP catalog JSON");
    let catalog = serde_json::to_string(&catalog).expect("Python MCP catalog literal");
    let capabilities = json!({
        "profile":"game-information-query-v1",
        "query_kinds":["list"], "entity_kinds":["card"],
        "projections":["summary"], "detail_levels":["summary"], "fields":["display_name"],
        "limits":{"page_items":4,"item_bytes":4096,"page_bytes":65536,"text_bytes":4096},
        "max_message_bytes":262144,"max_cursor_bytes":512,
        "snapshot_policy":{
            "supports_live":true,"lifetime_generations":128,"max_retained_snapshots":8,
            "expiry_behavior":"reject_stale_snapshot",
            "invalidated_by":["content_change","epoch_change","profile_change","restore","restart","run_change"]
        }
    });
    let capabilities = serde_json::to_string(&capabilities).expect("MCP capability JSON");
    let capabilities = serde_json::to_string(&capabilities).expect("Python MCP capability literal");
    let settled = serde_json::to_string(&dispatch_response()).expect("settled dispatch JSON");
    let settled = serde_json::to_string(&settled).expect("Python settled dispatch literal");
    format!(
        r#"#!/usr/bin/python3
import json, os, sys
LOG={log}
INITIAL=json.loads({initial})
CATALOG=json.loads({catalog})
CAPS=json.loads({capabilities})
SETTLED=json.loads({settled})
def emit(value):
    print(json.dumps(value,separators=(',',':'),sort_keys=True),flush=True)
def envelope(i, body, is_error=False):
    return {{"jsonrpc":"2.0","id":i,"result":{{"isError":is_error,"content":[{{"type":"text","text":json.dumps(body,separators=(',',':'),sort_keys=True)}}]}}}}
def secrets_absent():
    names=["STS2_LOOKUP_OWNER_CONFIG","STS2_LOOKUP_OWNER_CONFIG_SHA256",
           "STS2_LOOKUP_CORPUS_STORE_KEY_HEX","STS2_LOOKUP_POLICY_STORE_KEY_HEX",
           "STS2_LOOKUP_ARCHIVE_STORE_KEY_HEX","STS2_WORKFLOW_TOKEN_LOOKUP_OWNER"]
    return not any(name in os.environ for name in names)
def query_response(i, a):
    query={{
      "query_kind":"list","entity_kind":"card",
      "target":{{"definition_ref":a.get("definition_ref"),"instance_ref":None}},
      "filters":{{"display_name":a.get("display_name"),"namespaced_ids":a["namespaced_ids"],
                 "definition_refs":a["definition_refs"],"instance_ids":a["instance_ids"]}},
      "projection":a["projection"],"detail_level":a["detail_level"],"fields":a["fields"],
      "binding":{{"mode":"static","content_manifest_id":a["content_manifest_id"],
                  "locale":a["locale"],"visibility_scope":"public","instance_ref":None,"snapshot_ref":None}},
      "parent_observation":None,
      "limits":{{"page_items":a["page_items"],"item_bytes":a["item_bytes"],
                "page_bytes":a["page_bytes"],"text_bytes":a["text_bytes"]}},
      "cursor":a["cursor"]
    }}
    page={{"items":[],"final_page":True,"next_cursor":None,"cursor_binding":None,
          "coverage":"complete","total_count_known":True,"total_count":0,
          "ordering":{{"key":"definition_ref","direction":"ascending",
                       "algorithm":"identity_bytes","deterministic":True}},
          "limits":query["limits"]}}
    bare=json.dumps(page,separators=(',',':'),sort_keys=True).encode()
    page["accounting"]={{"item_count":0,"item_bytes":0,"payload_bytes":2,
                        "page_bytes":len(bare),"text_bytes":0}}
    return {{"protocol_version":"game-information-query-v1",
       "schema_digest":"376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9",
       "provenance":{{"artifact":"sts2-protocol/game-information-query-v1",
          "source":"schemas/game-information-query-v1.schema.json","generator":"hand-authored"}},
       "correlation_id":str(i),"kind":"query_response","query":query,
       "result":{{"read_only":True,"parent_observation":None,"result_generation":None,"page":page}},
       "capabilities":None,"error":None}}
for line in sys.stdin:
    req=json.loads(line); i=req.get("id"); method=req.get("method")
    params=req.get("params",{{}})
    if method=="initialize":
        emit({{"jsonrpc":"2.0","id":i,"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},
              "serverInfo":{{"name":"synthetic-entry-mcp","version":"1"}}}}}})
        continue
    if method=="tools/list":
        emit({{"jsonrpc":"2.0","id":i,"result":CATALOG}})
        continue
    args=params.get("arguments",{{}})
    name=params.get("name")
    with open(LOG,"a",encoding="utf-8") as out:
        out.write(json.dumps({{"tool":name,"owner_secrets_absent":secrets_absent()}})+"\n")
    if name=="sts2.observe":
        body=INITIAL.copy(); body["correlation_id"]=str(i)
    elif name=="sts2.legal_actions":
        body=INITIAL.copy(); body["kind"]="legal_actions_response"; body["correlation_id"]=str(i)
        body["observation"]=None
    elif name=="sts2.game_information_capabilities":
        body={{"protocol_version":"game-information-query-v1",
          "schema_digest":"376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9",
          "provenance":{{"artifact":"sts2-protocol/game-information-query-v1",
            "source":"schemas/game-information-query-v1.schema.json","generator":"hand-authored"}},
          "correlation_id":str(i),"kind":"capabilities_response","query":None,"result":None,
          "capabilities":CAPS,"error":None}}
    elif name=="sts2.game_information_list":
        body=query_response(i,args)
    elif name=="sts2.dispatch_action":
        body=SETTLED.copy(); body["correlation_id"]=str(i)
        body["instance_id"]=args["instance_id"]; body["session_id"]="session-1"
        body["lease_id"]=args["lease_id"]; body["lease_epoch"]=args["lease_epoch"]
        body["generation"]=args["generation"]+1; body["state_id"]=args["state_id"]
        body["operation_id"]=args["operation_id"]
        body["observation"]["state_id"]=args["state_id"]
        body["observation"]["generation"]=args["generation"]+1
        body["observation"]["state"]={{"state":"victory"}}
        body["legal_actions"]=[]
        body["transition"]={{"from_generation":args["generation"],"to_generation":args["generation"]+1,
          "state_id":args["state_id"],"effect_kind":"end_turn.settled"}}
    elif name=="sts2.wait_for_transition":
        body=SETTLED.copy(); body["kind"]="wait_response"; body["correlation_id"]=str(i)
        body["operation_id"]=args["operation_id"]; body["wait_outcome"]="successor"
    else:
        raise RuntimeError("unexpected synthetic MCP tool: "+str(name))
    emit(envelope(i,body))
"#
    )
}

pub(super) fn agent_script_source(log: &std::path::Path) -> String {
    let log = serde_json::to_string(&log.to_string_lossy()).expect("Python agent log literal");
    format!(
        r#"#!/usr/bin/python3
import json, os, sys
LOG={log}
def secrets_absent():
    names=["STS2_LOOKUP_OWNER_CONFIG","STS2_LOOKUP_OWNER_CONFIG_SHA256",
           "STS2_LOOKUP_CORPUS_STORE_KEY_HEX","STS2_LOOKUP_POLICY_STORE_KEY_HEX",
           "STS2_LOOKUP_ARCHIVE_STORE_KEY_HEX","STS2_WORKFLOW_TOKEN_LOOKUP_OWNER"]
    return not any(name in os.environ for name in names)
raw=input()
with open(LOG,"a",encoding="utf-8") as out:
 out.write(json.dumps({{"kind":"start","owner_secrets_absent":secrets_absent()}})+"\n")
frame=json.loads(raw)
request=frame["payload"]["request"]
args={{"operation_id":"entry-query","mode":"static",
 "query":{{"query_kind":"list","entity_kind":"card",
  "target":{{"definition_ref":None}},
  "filters":{{"display_name":None,"namespaced_ids":[],"definition_refs":[],"instance_ids":[]}},
  "projection":"summary","detail_level":"summary","fields":["display_name"],
  "limits":{{"page_items":4,"item_bytes":4096,"page_bytes":65536,"text_bytes":4096}},
  "cursor":None}}}}
frame["sequence"]=1
frame["payload"]={{"kind":"query","arguments":args}}
print(json.dumps(frame),flush=True)
feedback=json.loads(input())
value=feedback["payload"]["value"]
assert "data" in value
with open(LOG,"a",encoding="utf-8") as out:
 out.write(json.dumps({{"kind":"data","owner_secrets_absent":secrets_absent(),
                        "data_authority":value["data"].get("authority")}})+"\n")
frame["sequence"]=2
frame["payload"]={{"kind":"decision","action_id":request["legal_action_ids"][0]}}
with open(LOG,"a",encoding="utf-8") as out:
 out.write(json.dumps({{"kind":"decision","owner_secrets_absent":secrets_absent(),
                        "action_id":request["legal_action_ids"][0]}})+"\n")
print(json.dumps(frame),flush=True)
"#
    )
}

use super::gateway::{dispatch_response, state_response};
