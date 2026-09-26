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
            {"name":"sts2.game_information_list","annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true},"inputSchema":{"additionalProperties":false},"_meta":{"sts2":{"revision":"game-information-query-v1-mcp","feature":"static_reference"}}},
            {"name":"sts2.game_information.live_observation_bootstrap","annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true},"inputSchema":{"additionalProperties":false},"_meta":{"sts2":{"revision":"game-information-live-observation-bootstrap-v1","feature":"live_details"}}}
        ]
    });
    let catalog = serde_json::to_string(&catalog).expect("MCP catalog JSON");
    let catalog = serde_json::to_string(&catalog).expect("Python MCP catalog literal");
    let capabilities = json!({
        "profile":"game-information-query-v1",
        "query_kinds":["list","detail"], "entity_kinds":["card"],
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
import http.client
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
startup_request=os.environ.get("STS2_LOOKUP_BINDING_DISCOVERY_REQUEST_JSON")
with open(LOG,"a",encoding="utf-8") as out:
    out.write(json.dumps({{"kind":"startup","owner_secrets_absent":secrets_absent(),
                          "lookup_discovery_request":json.loads(startup_request) if startup_request else None}})+"\n")
def query_response(i, a):
    live=a.get("instance_ref") is not None
    query={{
      "query_kind":"detail" if live else "list","entity_kind":"card",
      "target":{{"definition_ref":a.get("definition_ref"),"instance_ref":a.get("instance_ref")}},
      "filters":{{"display_name":a.get("display_name"),"namespaced_ids":a["namespaced_ids"],
                 "definition_refs":a["definition_refs"],"instance_ids":a["instance_ids"]}},
      "projection":a["projection"],"detail_level":a["detail_level"],"fields":a["fields"],
      "binding":{{"mode":"live" if live else "static","content_manifest_id":a["content_manifest_id"],
                  "locale":a["locale"],"visibility_scope":"player" if live else "public",
                  "instance_ref":a.get("instance_ref"),"snapshot_ref":a.get("snapshot_ref")}},
      "parent_observation":a.get("parent_observation") if live else None,
      "limits":{{"page_items":a["page_items"],"item_bytes":a["item_bytes"],
                "page_bytes":a["page_bytes"],"text_bytes":a["text_bytes"]}},
      "cursor":a["cursor"]
    }}
    if live:
        connection=http.client.HTTPConnection(os.environ["STS2_GATEWAY_ADDR"],timeout=5)
        body={{"query":query,"correlation_id":str(i)}}
        encoded=json.dumps(body,separators=(',',':')).encode()
        # The Gateway refuses any header outside its closed allow-list, and CPython's
        # `HTTPConnection.request` injects `Accept-Encoding: identity` on its own for
        # every HTTP/1.1 request. Suppress that default rather than naming the header
        # ourselves: an explicit `Accept-Encoding` would still put the refused name on
        # the wire (the Gateway tests the name, not the value), so only suppression
        # states the guarantee this peer actually needs -- no header reaches the
        # Gateway that this script did not ask to send. (Refs #541, #547)
        connection.putrequest("POST","/v1/instances/"+a["instance_id"]+"/game-information/detail",
          skip_accept_encoding=True)
        connection.putheader("Content-Type","application/json")
        connection.putheader("Content-Length",str(len(encoded)))
        connection.putheader("x-mcp-session-id",a["mcp_session_id"])
        connection.putheader("x-sts2-instance-id",a["instance_id"])
        connection.putheader("x-sts2-session-id",os.environ["STS2_SESSION_ID"])
        connection.putheader("x-sts2-lease-id",a["lease_id"])
        connection.putheader("x-sts2-lease-epoch",str(a["lease_epoch"]))
        connection.putheader("x-sts2-correlation-id",str(i))
        connection.endheaders(encoded)
        return json.loads(connection.getresponse().read())
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
       "result":{{"read_only":True,"parent_observation":a.get("parent_observation") if live else None,
                  "result_generation":a.get("snapshot_ref",{{}}).get("state_generation") if live else None,"page":page}},
       "capabilities":None,"error":None}}
def bootstrap_proxy(i, a):
    request={{
      "protocol_version":"game-information-live-observation-bootstrap-v1",
      "schema_digest":"6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2",
      "provenance":{{"artifact":"sts2-protocol/game-information-live-observation-bootstrap-v1",
        "source":"schemas/game-information-live-observation-bootstrap-v1.schema.json","generator":"hand-authored"}},
      "correlation_id":str(i),"kind":"bootstrap_request",
      "selector":{{"definition_ref":a["definition_ref"],"instance_ref":a.get("instance_ref")}},
      "scope":{{"instance_id":a["instance_id"],"run_id":"run","authority_epoch":1,
        "content_manifest_id":a["content_manifest_id"],"locale":a["locale"]}},
      "limits":{{"max_visible_entities":64,"max_item_bytes":65536,"max_message_bytes":262144}},
      "parent_observation":None,"visible_entities":None,"owner_provenance":None,"error":None}}
    connection=http.client.HTTPConnection(os.environ["STS2_GATEWAY_ADDR"],timeout=5)
    body=json.dumps(request,separators=(',',':')).encode()
    # See `query_response`: CPython injects `Accept-Encoding: identity` unless this
    # request opts out, and the Gateway's closed allow-list has no such entry, so a
    # correct caller has to suppress the library default rather than name the header.
    # (Refs #541, #547)
    connection.putrequest("POST","/v1/instances/"+a["instance_id"]+"/game-information/live-observation-bootstrap",
      skip_accept_encoding=True)
    connection.putheader("Content-Type","application/json")
    connection.putheader("Content-Length",str(len(body)))
    connection.putheader("x-mcp-session-id",a["mcp_session_id"])
    connection.putheader("x-sts2-instance-id",a["instance_id"])
    connection.putheader("x-sts2-session-id",os.environ["STS2_SESSION_ID"])
    connection.putheader("x-sts2-lease-id",a["lease_id"])
    connection.putheader("x-sts2-lease-epoch",str(a["lease_epoch"]))
    connection.putheader("x-sts2-correlation-id",str(i))
    connection.endheaders(body)
    return json.loads(connection.getresponse().read())
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
    elif name in ("sts2.game_information_list","sts2.game_information_detail"):
        body=query_response(i,args)
    elif name=="sts2.game_information.live_observation_bootstrap":
        body=bootstrap_proxy(i,args)
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
    if name=="sts2.wait_for_transition":
        break
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
    return not any(name in os.environ for name in names) and \
           "STS2_LOOKUP_BINDING_DISCOVERY_REQUEST_JSON" not in os.environ
def log(event):
 with open(LOG,"a",encoding="utf-8") as out:
  out.write(json.dumps(event)+"\n")
def fail(kind, detail):
 # Surface the provider-side reason in the test log instead of a silent exit.
 log({{"kind":kind,"owner_secrets_absent":secrets_absent(),"detail":str(detail)[:512]}})
 sys.exit(1)
sys.excepthook=lambda kind, value, trace: fail("exception", value)
raw=input()
log({{"kind":"start","owner_secrets_absent":secrets_absent()}})
frame=json.loads(raw)
request=frame["payload"]["request"]
args={{"operation_id":"entry-query","mode":"live",
 "query":{{"query_kind":"detail","entity_kind":"card",
  "target":{{"definition_ref":{{"content_manifest_id":"content-1","entity_kind":"card",
   "namespaced_id":"ironclad:strike","variant":None}}}},
  "filters":{{"display_name":None,"namespaced_ids":[],"definition_refs":[],"instance_ids":[]}},
  "projection":"summary","detail_level":"summary","fields":["display_name"],
  "limits":{{"page_items":4,"item_bytes":4096,"page_bytes":65536,"text_bytes":4096}},
  "cursor":None}}}}
frame["sequence"]=1
frame["wire_version"]="sts2.exo-lookup-wire-v2-bootstrap"
bootstrap={{"operation_id":"entry-bootstrap",
 "definition_ref":{{"content_manifest_id":"content-1","entity_kind":"card",
  "namespaced_id":"ironclad:strike","variant":None}},"instance_ref":None}}
frame["payload"]={{"kind":"bootstrap","arguments":bootstrap}}
print(json.dumps(frame),flush=True)
feedback=json.loads(input())
value=feedback["payload"]["value"]
if "error" in value:
 log({{"kind":"error","owner_secrets_absent":secrets_absent(),"error":value["error"]}})
 sys.exit(1)
if "bootstrap" not in value:
 fail("unexpected_feedback", json.dumps(feedback)[:400])
frame["sequence"]=2
frame["wire_version"]="sts2.exo-lookup-wire-v1"
frame["payload"]={{"kind":"query","arguments":args}}
print(json.dumps(frame),flush=True)
feedback=json.loads(input())
value=feedback["payload"]["value"]
if "error" in value:
 log({{"kind":"error","owner_secrets_absent":secrets_absent(),"error":value["error"]}})
 sys.exit(1)
if "data" not in value:
 fail("unexpected_feedback", json.dumps(feedback)[:400])
with open(LOG,"a",encoding="utf-8") as out:
 out.write(json.dumps({{"kind":"data","owner_secrets_absent":secrets_absent(),
                        "data_authority":value["data"].get("authority")}})+"\n")
frame["sequence"]=3
frame["payload"]={{"kind":"decision","action_id":request["legal_action_ids"][0]}}
with open(LOG,"a",encoding="utf-8") as out:
 out.write(json.dumps({{"kind":"decision","owner_secrets_absent":secrets_absent(),
                        "action_id":request["legal_action_ids"][0]}})+"\n")
print(json.dumps(frame),flush=True)
"#
    )
}

use super::gateway::{dispatch_response, state_response};
