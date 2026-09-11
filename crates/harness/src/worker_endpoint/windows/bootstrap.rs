// SPDX-License-Identifier: MIT

use crate::worker_handoff::{
    AuthenticatedWorkerRequest, WorkerCapability, WorkerCommand, WorkerRequest,
};
use crate::worker_runtime::{
    ResponseWriteStatus, WorkerExecutionCompletion, WorkerExecutionTask, WorkerRuntime,
    WorkerStartOutcome,
};
use crate::{
    ExecutionFingerprint, ExecutionStore, ExecutionStoreConfig, WorkerBoot, WorkerOwnerProof,
};
use std::ffi::OsString;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const BOOTSTRAP_MAGIC: &[u8; 8] = b"ASC-WB01";
const BOOTSTRAP_FRAME_PREFIX_BYTES: usize = 12;
const MAX_PAYLOAD: usize = 16 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 4 * 1024;
const TRANSPORT_MAX_FRAME_BYTES: usize = crate::worker_handoff::MAX_FRAME_BYTES;
const AUTH_MAGIC: &[u8] = b"ascension-worker-auth-v1\0";
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_AUTH_SLOTS: usize = 4;
const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

pub fn run_from_environment() -> Result<(), String> {
    let bootstrap = read_bootstrap()?;
    let config = EndpointConfig::from_environment(&bootstrap)?;
    let boot_id = fresh_worker_boot_id(&bootstrap.watchdog_boot_id);
    let boot = WorkerBoot::new(
        config.deployment_id.clone(),
        config.worker_owner_id.clone(),
        config.worker_profile_digest.clone(),
        boot_id.clone(),
    )
    .map_err(|_| String::from("worker boot identity is invalid"))?;
    let mut store = ExecutionStore::open(
        ExecutionStoreConfig::new(config.execution_store_path.clone())
            .with_approved_recovery_schema(),
    )
    .map_err(|_| String::from("worker execution store is unavailable"))?;
    store
        .start_worker_boot(&boot)
        .map_err(|_| String::from("worker boot could not be persisted"))?;
    let shared_store = crate::worker_runtime_store::share_store(store);
    let command = crate::worker_handoff::WorkerCommandConfig::new(
        config.deployment_id.clone(),
        config.worker_owner_id.clone(),
        config.worker_profile_digest.clone(),
        config.release_digest.clone(),
        config.config_digest.clone(),
        boot_id.clone(),
        bootstrap.watchdog_boot_id.clone(),
    )
    .map_err(|_| String::from("worker command binding is invalid"))?;
    let mut runtime = WorkerRuntime::from_shared_store(
        shared_store,
        command,
        config.fingerprint.clone(),
        boot_id,
    )
    .map_err(|_| String::from("worker runtime could not be initialized"))?;
    let endpoint = derive_endpoint(&config.endpoint_namespace, &bootstrap.launch_nonce)?;
    let expected = sts2_harness_windows_boundary::PeerExpectation::new(
        bootstrap.peer.pid,
        bootstrap.peer.creation_token,
        bootstrap.peer.executable.clone(),
        bootstrap.peer.executable_sha256.clone(),
        bootstrap.peer.session_id,
        bootstrap.peer.sid.clone(),
    )?;
    let listener = sts2_harness_windows_boundary::WorkerPipeListener::create(&endpoint, expected)?;
    serve(listener, &config, &bootstrap, &mut runtime)
}

struct EndpointConfig {
    endpoint_namespace: PathBuf,
    credential_path: PathBuf,
    execution_store_path: PathBuf,
    deployment_id: String,
    worker_owner_id: String,
    worker_profile_digest: String,
    release_digest: String,
    config_digest: String,
    fingerprint: ExecutionFingerprint,
    runtime_executable: ApprovedExecutable,
    environment: Vec<(OsString, OsString)>,
}

impl EndpointConfig {
    fn from_environment(bootstrap: &Bootstrap) -> Result<Self, String> {
        let endpoint_namespace = required_path("STS2_WORKER_ENDPOINT_NAMESPACE")?;
        validate_namespace(&endpoint_namespace)?;
        let credential_path = required_path("STS2_WORKER_CREDENTIAL_PATH")?;
        validate_reference(&credential_path, "worker credential")?;
        let execution_store_path = required_path("STS2_EXECUTION_STORE_PATH")?;
        validate_reference(&execution_store_path, "worker execution store")?;
        if credential_path == execution_store_path {
            return Err(String::from("worker credential and store must be distinct"));
        }
        let deployment_id = alias_env("STS2_WORKER_DEPLOYMENT_ID", "STS2_DEPLOYMENT_ID")?
            .ok_or_else(|| String::from("STS2_WORKER_DEPLOYMENT_ID is required"))?;
        let worker_owner_id = alias_env("STS2_WORKER_OWNER_ID", "STS2_WORKER_OWNER")?
            .unwrap_or_else(|| bootstrap.component_id.clone());
        if worker_owner_id != bootstrap.component_id {
            return Err(String::from(
                "worker owner identity does not match the bootstrap component",
            ));
        }
        let worker_profile_digest = required_env("STS2_WORKER_PROFILE_DIGEST")?;
        let release_digest = alias_env("STS2_WORKER_RELEASE_DIGEST", "STS2_BUILD_DIGEST")?
            .ok_or_else(|| String::from("STS2_WORKER_RELEASE_DIGEST is required"))?;
        let config_digest = alias_env("STS2_WORKER_CONFIG_DIGEST", "STS2_RUNTIME_CONFIG_DIGEST")?
            .ok_or_else(|| String::from("STS2_WORKER_CONFIG_DIGEST is required"))?;
        for (name, value) in [
            ("STS2_WORKER_PROFILE_DIGEST", &worker_profile_digest),
            ("STS2_WORKER_RELEASE_DIGEST", &release_digest),
            ("STS2_WORKER_CONFIG_DIGEST", &config_digest),
        ] {
            validate_digest(value).map_err(|_| format!("{name} is invalid"))?;
        }
        let seed = alias_env("STS2_WORKER_SEED", "STS2_SEED")?
            .ok_or_else(|| String::from("STS2_WORKER_SEED or STS2_SEED is required"))?;
        let state = alias_env("STS2_WORKER_STATE_DIGEST", "STS2_STATE_DIGEST")?
            .ok_or_else(|| String::from("STS2_WORKER_STATE_DIGEST is required"))?;
        let provider = alias_env("STS2_WORKER_PROVIDER_DIGEST", "STS2_PROVIDER_DIGEST")?
            .ok_or_else(|| String::from("STS2_WORKER_PROVIDER_DIGEST is required"))?;
        let fingerprint = ExecutionFingerprint::new(
            seed,
            release_digest.clone(),
            state,
            config_digest.clone(),
            provider,
        )
        .map_err(|_| String::from("worker execution fingerprint is invalid"))?;
        let runtime_binary = optional_path("STS2_WORKER_RUNTIME_BINARY")?
            .unwrap_or(current_runtime_binary()?);
        let expected_runtime_digest = optional_env("STS2_WORKER_RUNTIME_SHA256")?
            .or_else(|| Some(release_digest.clone()));
        let runtime_executable =
            verify_executable(&runtime_binary, expected_runtime_digest.as_deref())?;
        Ok(Self {
            endpoint_namespace,
            credential_path,
            execution_store_path,
            deployment_id,
            worker_owner_id,
            worker_profile_digest,
            release_digest,
            config_digest,
            fingerprint,
            runtime_executable,
            environment: approved_environment(),
        })
    }
}

struct Bootstrap {
    launch_nonce: String,
    watchdog_boot_id: String,
    component_id: String,
    peer: WindowsPeer,
}

#[derive(Clone)]
struct WindowsPeer {
    pid: u32,
    creation_token: u64,
    executable: PathBuf,
    executable_sha256: String,
    session_id: u32,
    sid: String,
}

fn read_bootstrap() -> Result<Bootstrap, String> {
    let frame = sts2_harness_windows_boundary::read_bootstrap_stdin(
        BOOTSTRAP_MAGIC,
        MAX_PAYLOAD,
        BOOTSTRAP_TIMEOUT,
    )?;
    let payload = frame
        .get(BOOTSTRAP_FRAME_PREFIX_BYTES..)
        .ok_or_else(|| String::from("worker bootstrap frame is truncated"))?;
    parse_bootstrap_payload(payload)
}
