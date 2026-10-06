// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::seed_v2_support::{
    CountingAuthority, PrivateDirectory, derive_once_request, open_store, runtime_counts,
    start_live_server,
};
use crate::management::{
    ManagementClient, SeedBindingStateV2, SeededRunSubmissionResponseV2, WorkflowRunRequestV2,
    WorkflowStore,
};

const MODE: &str = "HARNESS_SEED_V2_PROCESS_MODE";
const DATABASE: &str = "HARNESS_SEED_V2_PROCESS_DATABASE";
const REQUEST: &str = "HARNESS_SEED_V2_PROCESS_REQUEST";
const RESPONSE: &str = "HARNESS_SEED_V2_PROCESS_RESPONSE";
const KEYRING: &str = "HARNESS_SEED_V2_PROCESS_KEYRING";
const CATALOG: &str = "HARNESS_SEED_V2_PROCESS_CATALOG";

#[cfg(target_os = "linux")]
#[test]
fn independent_process_restart_replays_durable_candidate_without_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("workflow.sqlite");
    let request_path = directory.path().join("request.json");
    let first_response_path = directory.path().join("first-process-response.json");
    let replay_response_path = directory.path().join("replay-process-response.json");
    let key_v1 = directory.keyring("keys-v1.conf", "key-1", &[("key-1", "11")]);
    let key_rotated = directory.keyring(
        "keys-rotated.conf",
        "key-2",
        &[("key-1", "11"), ("key-2", "22")],
    );
    let (request, _) = derive_once_request();
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("serialize request"),
    )
    .expect("persist subprocess request fixture");

    let first_process = run_child(
        "submit",
        &database,
        &request_path,
        &first_response_path,
        &key_v1,
        "live.catalog.v1",
    )?;
    assert_success(&first_process, "initial process submission");

    let replay_process = run_child(
        "replay",
        &database,
        &request_path,
        &replay_response_path,
        &key_rotated,
        "live.catalog.rotated",
    )?;
    assert_success(&replay_process, "independent process replay");
    eprintln!(
        "harness103 SQLite process witness: submit child pid={} exit={:?}; replay child pid={} exit={:?}",
        first_process.process_id,
        first_process.output.status.code(),
        replay_process.process_id,
        replay_process.output.status.code()
    );
    let first_response: SeededRunSubmissionResponseV2 = serde_json::from_slice(
        &std::fs::read(&first_response_path).expect("read first process response"),
    )
    .expect("decode first process response");
    let response: SeededRunSubmissionResponseV2 = serde_json::from_slice(
        &std::fs::read(&replay_response_path).expect("read replay response from child process"),
    )
    .expect("decode replay response");
    assert_eq!(
        response.run.workflow_run_id,
        first_response.run.workflow_run_id
    );
    assert_eq!(
        response.seed_binding.operation_id,
        first_response.seed_binding.operation_id
    );
    assert_eq!(
        response.seed_binding.effective_seed,
        first_response.seed_binding.effective_seed
    );
    assert_eq!(
        response.seed_binding.configuration_digest,
        first_response.seed_binding.configuration_digest
    );
    assert_eq!(
        response.seed_binding.algorithm_id,
        first_response.seed_binding.algorithm_id
    );
    assert_eq!(
        response.seed_binding.key_authority_id,
        first_response.seed_binding.key_authority_id
    );
    assert_eq!(
        response.seed_binding.key_version,
        first_response.seed_binding.key_version
    );
    assert!(response.seed_binding.state == first_response.seed_binding.state);
    assert!(response.seed_binding.state == SeedBindingStateV2::AwaitingHostContext);

    let reopened = open_store(&database);
    let record = reopened
        .read_seed_binding(&response.run.workflow_run_id)
        .unwrap()
        .expect("persisted seed binding after both processes exit");
    assert_eq!(
        record.record().operation_id,
        response.seed_binding.operation_id
    );
    assert_eq!(
        record.record().effective_seed,
        response.seed_binding.effective_seed
    );
    assert_eq!(
        reopened
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM management_seed_bindings", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1,
        "the replay process observes one immutable durable winner"
    );
    drop(reopened);
    directory.cleanup();
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn seed_v2_process_child_entry() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(mode) = std::env::var(MODE) else {
        return Ok(());
    };
    assert!(
        mode == "submit" || mode == "replay",
        "unknown child test mode"
    );
    let database = required_env(DATABASE)?;
    let request_path = required_env(REQUEST)?;
    let request: WorkflowRunRequestV2 =
        serde_json::from_slice(&std::fs::read(&request_path).expect("read parent request fixture"))
            .expect("decode parent request fixture");
    let keys = Arc::new(CountingAuthority::open(Path::new(&required_env(KEYRING)?)));
    let store = open_store(Path::new(&database));
    let server = start_live_server(
        Arc::clone(&store),
        Arc::clone(&keys),
        &required_env(CATALOG)?,
    );
    let response = ManagementClient::new(
        server.server.address(),
        super::super::seed_v2_support::TOKEN,
    )
    .expect("child process client")
    .request_json(
        "POST",
        "/v2/workflow-runs",
        Some(&serde_json::to_vec(&request).expect("encode child request")),
    )
    .expect("submit from child process");
    assert_eq!(
        response.status, 200,
        "served child response: {:?}",
        response.body
    );
    let response: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&response.body).expect("decode served child response");
    assert!(response.seed_binding.state == SeedBindingStateV2::AwaitingHostContext);
    assert_eq!(runtime_counts(&server.runtime_counters), (0, 0, 0, 0));
    if mode == "replay" {
        assert_eq!(
            keys.current_reads(),
            0,
            "replay does not select current key"
        );
        assert!(
            keys.pinned_reads() > 0,
            "replay selects pinned historical key"
        );
        assert_eq!(
            server
                .catalog_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "replay bypasses the rotated current catalog"
        );
    }
    std::fs::write(
        required_env(RESPONSE)?,
        serde_json::to_vec(&response).unwrap(),
    )
    .expect("write child process response");
    server.server.shutdown().expect("stop child process server");
    drop(store);
    Ok(())
}

struct ChildResult {
    process_id: u32,
    output: Output,
}

fn run_child(
    mode: &str,
    database: &Path,
    request: &Path,
    response: &Path,
    keyring: &Path,
    catalog: &str,
) -> Result<ChildResult, std::io::Error> {
    let child = Command::new(std::env::current_exe()?)
        .arg("seed_v2_process_child_entry")
        .arg("--nocapture")
        .env(MODE, mode)
        .env(DATABASE, database)
        .env(REQUEST, request)
        .env(RESPONSE, response)
        .env(KEYRING, keyring)
        .env(CATALOG, catalog)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let process_id = child.id();
    let deadline = Instant::now() + Duration::from_secs(30);
    let output = wait_for_child(child, deadline, process_id)?;
    Ok(ChildResult { process_id, output })
}

fn wait_for_child(
    mut child: Child,
    deadline: Instant,
    process_id: u32,
) -> Result<Output, std::io::Error> {
    loop {
        let (error_kind, reason) = match child.try_wait() {
            Ok(Some(_)) => {
                return child.wait_with_output().map_err(|error| {
                    std::io::Error::other(format!(
                        "collect output from owned child process {process_id}: {error}"
                    ))
                });
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Ok(None) => (
                std::io::ErrorKind::TimedOut,
                format!("owned child process {process_id} exceeded 30s"),
            ),
            Err(error) => (
                std::io::ErrorKind::Other,
                format!("could not observe owned child process {process_id}: {error}"),
            ),
        };
        let kill = child.kill();
        let output = child.wait_with_output().map_err(|error| {
            std::io::Error::other(format!(
                "{reason}; wait for owned child process {process_id} after kill {kill:?}: {error}"
            ))
        })?;
        return Err(std::io::Error::new(
            error_kind,
            format!(
                "{reason}; kill result {kill:?}; stdout={}; stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
}

fn assert_success(result: &ChildResult, label: &str) {
    assert!(
        result.output.status.success(),
        "{label} failed in owned child {} with {:?}; stdout={}; stderr={}",
        result.process_id,
        result.output.status.code(),
        String::from_utf8_lossy(&result.output.stdout),
        String::from_utf8_lossy(&result.output.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.output.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;")),
        "{label} child {} did not execute exactly one selected test: {stdout}",
        result.process_id
    );
}

fn required_env(name: &str) -> Result<String, std::io::Error> {
    std::env::var(name).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("missing child process input {name}: {error}"),
        )
    })
}
