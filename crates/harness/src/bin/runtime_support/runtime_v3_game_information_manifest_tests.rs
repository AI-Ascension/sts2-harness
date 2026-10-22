// SPDX-License-Identifier: MIT
//! The real runtime MCP process and owner fence, using only a synthetic subprocess peer.
use super::*;
use super::lookup_binding_tests::config;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use crate::runtime_support::runtime_v3::game_information_owner::owner_management_tests::{
    AdoptedRuntimeOwnerFixture, adopted_runtime_owner,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn peer(
    fixture: &AdoptedRuntimeOwnerFixture,
    mode: &str,
) -> Result<(RuntimeV3Port, Directory), Box<dyn std::error::Error>> {
    let directory = Directory(std::env::temp_dir().join(format!(
        "sts2-manifest-mcp-{}", uuid::Uuid::new_v4()
    )));
    std::fs::create_dir(&directory.0)?;
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700))?;
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/game-information-content-manifest-v1/golden/canonical-manifest-response.json"
    ))?;
    if mode == "typed" {
        value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/game-information-content-manifest-v1/golden/access-denied-error-response.json"
        ))?;
    }
    if mode == "invalid" {
        value["schema_digest"] = json!("0".repeat(64));
    }
    let mut tools = ["observe", "legal_actions", "dispatch_action", "wait_for_transition", "reobserve", "recover", "capabilities"]
        .map(|name| json!({"name":format!("sts2.{name}")})).to_vec();
    if mode != "missing" {
        tools.push(json!({"name":"sts2.game_information_content_manifest",
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true},
            "inputSchema":{"additionalProperties":false},
            "_meta":{"sts2":{"revision":"game-information-query-v1-mcp","feature":"static_reference"}}}));
    }
    let catalog = json!({"revision":"negotiated-composition-v1-mcp","refresh_required":mode=="stale",
        "session_epoch":1,"composition":{"revision":"negotiated-composition-v1-mcp"},"tools":tools});
    let script = format!(r#"#!/usr/bin/python3
import json,sys,pathlib,time
root=pathlib.Path(__file__).parent
catalog=json.loads({catalog:?})
value=json.loads({value:?})
for line in sys.stdin:
 request=json.loads(line)
 if request['method']=='tools/list':
  assert request['params']=={{}}
  result=catalog
 else:
  assert request['method']=='tools/call'
  assert request['params']=={{'name':'sts2.game_information_content_manifest','arguments':{{'instance_id':'instance-1','mcp_session_id':'mcp-session-1','lease_id':'lease-1','lease_epoch':1}}}}
  if {mode:?}=='blocked':
   (root/'started').write_text('started')
   deadline=time.monotonic()+3
   while not (root/'release').exists():
    if time.monotonic()>deadline: sys.exit(3)
    time.sleep(.005)
  value['correlation_id']=str(request['id'])
  result={{'isError':value['kind']=='error_response','content':[{{'type':'text','text':json.dumps(value)}}],'structuredContent':value}}
 print(json.dumps({{'jsonrpc':'2.0','id':request['id'],'result':result}}),flush=True)
"#, catalog=serde_json::to_string(&catalog)?, value=serde_json::to_string(&value)?);
    let executable = directory.0.join("peer.py");
    std::fs::write(&executable, script)?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
    let mut config = config("127.0.0.1:9".into());
    config.mcp_binary = executable.to_str().ok_or("synthetic executable path")?.to_owned();
    let mut port = RuntimeV3Port::new_with_telemetry(
        config,
        crate::runtime_support::runtime_v3_telemetry::TelemetryHandle::disabled(),
    )?;
    port.mcp = Some(crate::runtime_support::mcp_process::McpProcess::spawn(&port.config)?);
    port.lookup_binding_required = true;
    port.lookup_policy_owner = Some(fixture.owner.clone());
    port.lookup_policy_binding = Some(fixture.binding.clone());
    Ok((port, directory))
}

#[test]
fn manifest_runtime_negotiates_fresh_catalog_and_delivers_only_the_closed_response() -> TestResult {
    for mode in ["success", "typed", "invalid", "missing", "stale"] {
        let fixture = adopted_runtime_owner();
        let (mut port, directory) = peer(&fixture, mode)?;
        let result = port.information_content_manifest();
        match mode {
            "success" | "typed" => {
                let (correlation, bytes) = result?;
                assert_eq!(correlation, "2");
                let value: Value = serde_json::from_slice(&bytes)?;
                assert_eq!(value["kind"], if mode=="typed" {"error_response"} else {"content_manifest_response"});
                assert_eq!(value["correlation_id"], correlation);
            }
            "missing" | "stale" => assert_eq!(result, Err(LookupError::MissingCapability)),
            _ => assert_eq!(result, Err(LookupError::Invalid)),
        }
        assert_eq!(port.next_rpc_id, 3);
        drop(port);
        drop(directory);
    }
    Ok(())
}

#[test]
fn manifest_runtime_requires_current_owner_before_any_mcp_request() -> TestResult {
    let fixture = adopted_runtime_owner();
    let (mut port, directory) = peer(&fixture, "success")?;
    port.lookup_binding_required = false;
    assert_eq!(port.information_content_manifest(), Err(LookupError::Scope));
    port.lookup_binding_required = true;
    fixture.authority.update(|state| {
        state.control_epoch += 1;
        Ok(())
    })?;
    assert_eq!(port.information_content_manifest(), Err(LookupError::Scope));
    assert_eq!(port.next_rpc_id, 1);
    drop(port);
    drop(directory);
    Ok(())
}

#[test]
fn manifest_runtime_discards_response_when_owner_changes_during_mcp_read() -> TestResult {
    let fixture = adopted_runtime_owner();
    let (mut port, directory) = peer(&fixture, "blocked")?;
    let authority = fixture.authority.clone();
    let path = directory.0.clone();
    let revocation = std::thread::spawn(move || -> Result<bool, String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !path.join("started").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = path.join("started").exists();
        let revoked = authority.update(|state| {
            state.control_epoch += 1;
            Ok(())
        });
        std::fs::write(path.join("release"), b"release").map_err(|e| e.to_string())?;
        revoked.map_err(|e| e.to_string())?;
        Ok(started)
    });
    let result = port.information_content_manifest();
    let started = revocation.join().map_err(|_| "synthetic revocation panicked")??;
    assert!(started, "the production read reached the peer before revocation");
    assert_eq!(result, Err(LookupError::Scope));
    drop(port);
    Ok(())
}
