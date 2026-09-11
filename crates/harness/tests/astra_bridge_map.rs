// SPDX-License-Identifier: MIT

#![cfg(unix)]

use serde_json::{Value, json};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use sts2_harness::{EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES};

const MAP_SCHEMA_DIGEST: &str = "ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b";
const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn executable_bridge_accepts_a_valid_map_request_above_the_ordinary_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let marker = temporary.path().join("provider-invoked");
    let prompt = temporary.path().join("provider-prompt");
    let codex = temporary.write_executable(
        "codex",
        r#"#!/bin/sh
set -eu
output=
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--output-last-message" ]; then
        shift
        output=$1
    fi
    shift
done
cat > "$FAKE_CODEX_PROMPT"
if ! grep -F 'map-sentinel' "$FAKE_CODEX_PROMPT" >/dev/null; then
    exit 17
fi
touch "$FAKE_CODEX_MARKER"
printf '%s' '{"action_ids":["move-1"],"rationale":"large map accepted"}' > "$output"
printf '%s\n' '{"type":"thread.started","thread_id":"fake-map-thread"}' '{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1}}'
"#,
    )?;
    let request = large_map_request()?;
    assert!(request.len() > EXO_MAX_STANDARD_REQUEST_BYTES);
    assert!(request.len() <= EXO_MAX_MAP_REQUEST_BYTES);

    let output = run_bridge(&temporary, &codex, &marker, &prompt, &request, true)?;
    assert!(output.status.success(), "bridge failed: {output:?}");
    let decision: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(decision["decision"], "plan");
    assert_eq!(decision["action_ids"], json!(["move-1"]));
    assert!(
        fs::metadata(&marker).is_ok(),
        "fake provider was not invoked"
    );
    let prompt_bytes = fs::read(&prompt)?;
    assert!(
        prompt_bytes
            .windows(b"map-sentinel".len())
            .any(|window| { window == b"map-sentinel" })
    );
    Ok(())
}

#[test]
fn executable_bridge_rejects_an_over_bound_request_before_provider_launch()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let marker = temporary.path().join("provider-invoked");
    let prompt = temporary.path().join("provider-prompt");
    let codex = temporary.write_executable(
        "codex",
        r#"#!/bin/sh
set -eu
touch "$FAKE_CODEX_MARKER"
exit 17
"#,
    )?;
    let mut value: Value = serde_json::from_slice(&large_map_request()?)?;
    value["padding"] = Value::String("x".repeat(EXO_MAX_MAP_REQUEST_BYTES));
    let request = serde_json::to_vec(&value)?;
    assert!(request.len() > EXO_MAX_MAP_REQUEST_BYTES);

    let output = run_bridge(&temporary, &codex, &marker, &prompt, &request, false)?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.status.success());
    assert!(fs::metadata(&marker).is_err(), "provider was launched");
    Ok(())
}

fn run_bridge(
    temporary: &TempDir,
    codex: &Path,
    marker: &Path,
    prompt: &Path,
    request: &[u8],
    expect_provider: bool,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let bridge = env!("CARGO_BIN_EXE_sts2-astra-bridge");
    let system_path = env::var_os("PATH").unwrap_or_default();
    let path = format!(
        "{}:{}",
        codex.parent().ok_or("codex parent")?.display(),
        system_path.to_string_lossy()
    );
    let accounting = temporary.path().join("accounting.jsonl");
    let mut child = Command::new(bridge)
        .env("PATH", path)
        .env("FAKE_CODEX_MARKER", marker)
        .env("FAKE_CODEX_PROMPT", prompt)
        .env("STS2_PROVIDER_ACCOUNTING_PATH", &accounting)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let write_result = child.stdin.take().ok_or("bridge stdin")?.write_all(request);
    if expect_provider {
        write_result?;
    } else if let Err(error) = write_result
        && error.kind() != io::ErrorKind::BrokenPipe
    {
        return Err(error.into());
    }
    let output = child.wait_with_output()?;
    if expect_provider {
        let record = fs::read_to_string(accounting)?;
        assert!(record.contains("\"request_sha256\""));
    } else {
        assert!(fs::metadata(accounting).is_err());
    }
    Ok(output)
}

fn large_map_request() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut nodes = vec![json!({
        "id":"start", "row":0, "column":0, "category":"start", "visited":true
    })];
    let mut node_ids = vec![String::from("start")];
    for index in 1..256 {
        let id = format!("z{index:03}{}", "a".repeat(124));
        node_ids.push(id.clone());
        nodes.push(json!({
            "id":id, "row":index, "column":0, "category":"monster", "visited":false
        }));
    }
    let mut edges = Vec::new();
    'outer: for from in 0..node_ids.len() {
        for to in (from + 1)..node_ids.len() {
            edges.push(json!({"from":node_ids[from],"to":node_ids[to]}));
            if edges.len() == 600 {
                break 'outer;
            }
        }
    }
    let snapshot = json!({
        "state_id":"state-1", "generation":1, "schema_version":"visible-map-v1",
        "projection_version":"runtime-map-v1", "game_build":"build", "mod_version":"map-sentinel",
        "map_instance_id":"map-1", "act_id":1, "scope_id":"scope-1", "availability":"available",
        "completeness":"complete", "freshness":"current", "reason":null,
        "nodes":nodes, "edges":edges, "position":{"kind":"current","node_id":"start"},
        "history":["start"], "terminal_node_ids":[node_ids[255]],
        "bindings":[{"graph_node_id":node_ids[1],"host_action_id":"move-1",
            "action":{"kind":"select_map_node","node_id":node_ids[1]}}]
    });
    let snapshot_bytes = serde_json::to_vec(&snapshot)?;
    let snapshot_digest = sts2_harness::sha256_hex(&snapshot_bytes);
    let request = json!({
        "schema":"sts2.exo-decision-map-v1", "provider_revision":REVISION,
        "model_execution_id":"model-1", "state_id":"state-1", "generation":1,
        "observation":{
            "state_id":"state-1", "generation":1, "visible_seed":null,
            "player":{"hp":50,"max_hp":50,"energy":3,"gold":99,
                "hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"map","node_id":"start","options":[node_ids[1]]},
            "legal_actions":[{"action_id":"move-1",
                "action":{"kind":"select_map_node","node_id":node_ids[1]}}]
        },
        "legal_action_ids":["move-1"], "objective":"choose a legal map node",
        "hard_constraints":[], "max_response_bytes":8192,
        "map_context":{
            "profile":"runtime-map-v1", "schema_digest":MAP_SCHEMA_DIGEST,
            "snapshot_digest":snapshot_digest, "snapshot":snapshot
        }
    });
    Ok(serde_json::to_vec(&request)?)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> io::Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let path = Path::new("/tmp").join(format!(
            "sts2-astra-bridge-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_executable(&self, name: &str, content: &str) -> io::Result<PathBuf> {
        let path = self.path().join(name);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let mut file = options.open(&path)?;
        file.write_all(content.as_bytes())?;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions)?;
        Ok(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
