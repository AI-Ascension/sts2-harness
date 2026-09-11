// SPDX-License-Identifier: MIT

    use crate::worker_handoff::{
        AuthenticatedWorkerRequest, WorkerCapability, WorkerCommand, WorkerRequest,
    };
    use crate::worker_runtime::{
        ResponseWriteStatus, WorkerExecutionCompletion, WorkerExecutionTask, WorkerRuntime,
        WorkerStartOutcome,
    };
    use crate::worker_runtime_store::share_store;
    use crate::{
        ExecutionFingerprint, ExecutionStore, ExecutionStoreConfig, WorkerBoot, WorkerOwnerProof,
    };
    use rustix::fs::{
        MemfdFlags, Mode, OFlags, SealFlags, fcntl_add_seals, fstat, memfd_create, open,
    };
    use rustix::net::RecvAncillaryMessage;
    use rustix::net::sockopt::{set_socket_passcred, socket_peercred};
    use rustix::process::{Pid, PidfdFlags, geteuid, pidfd_open};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::ffi::OsString;
    use std::fs::{self, File, Permissions};
    use std::io::{ErrorKind, IoSliceMut, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Component, Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};
    use zeroize::Zeroizing;

    const BOOTSTRAP_MAGIC: &[u8; 8] = b"ASC-WB01";
    const BOOTSTRAP_MAX_PAYLOAD: usize = 16_384;
    const BOOTSTRAP_PREFIX_BYTES: usize = 12;
    const AUTH_MAGIC: &[u8] = b"ascension-worker-auth-v1\0";
    const MAX_CREDENTIAL_BYTES: usize = 4 * 1024;
    const MAX_PATH_BYTES: usize = 4 * 1024;
    const MAX_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
    const TRANSPORT_MAX_FRAME_BYTES: usize = crate::worker_handoff::MAX_FRAME_BYTES;
    const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(5);
    const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
    const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_AUTH_SLOTS: usize = 4;
    const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(5);

    /// Start the endpoint using only the static owner-approved environment and
    /// the dynamic bootstrap supplied on stdin.
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
        let shared_store = share_store(store);
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
        let listener = bind_endpoint(&config.endpoint_namespace, &endpoint)?;
        let _socket_guard = SocketGuard::new(endpoint)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| String::from("worker endpoint could not be made nonblocking"))?;
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
            if endpoint_namespace == credential_path || endpoint_namespace == execution_store_path {
                return Err(String::from("worker references must be distinct"));
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
            let config_digest =
                alias_env("STS2_WORKER_CONFIG_DIGEST", "STS2_RUNTIME_CONFIG_DIGEST")?
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

            let runtime_binary = match optional_path("STS2_WORKER_RUNTIME_BINARY")? {
                Some(path) => path,
                None => current_runtime_binary()?,
            };
            let expected_runtime_digest = optional_env("STS2_WORKER_RUNTIME_SHA256")?
                .or_else(|| Some(release_digest.clone()));
            let runtime_executable =
                verify_executable(&runtime_binary, expected_runtime_digest.as_deref())?;
            let environment = approved_environment();
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
                environment,
            })
        }
    }

    struct Bootstrap {
        launch_nonce: String,
        watchdog_boot_id: String,
        component_id: String,
        peer: LinuxPeer,
    }

    #[derive(Clone)]
    struct LinuxPeer {
        pid: u32,
        creation_token: String,
        executable: PathBuf,
        executable_sha256: String,
        uid: u32,
        gid: u32,
    }

    struct PeerImageProof {
        image: Arc<File>,
        image_identity: FileIdentity,
        digest: String,
    }

    struct PeerProofState {
        result: Mutex<Option<Result<Arc<PeerImageProof>, String>>>,
        ready: Condvar,
    }

    fn spawn_peer_image_proof(peer: &LinuxPeer) -> Result<Arc<PeerProofState>, String> {
        let state = Arc::new(PeerProofState {
            result: Mutex::new(None),
            ready: Condvar::new(),
        });
        let worker_state = Arc::clone(&state);
        let peer = peer.clone();
        thread::Builder::new()
            .name(String::from("sts2-worker-peer-proof"))
            .spawn(move || {
                let result = prove_peer_image(&peer).map(Arc::new);
                if let Ok(mut slot) = worker_state.result.lock() {
                    *slot = Some(result);
                    worker_state.ready.notify_all();
                }
            })
            .map_err(|_| String::from("worker peer proof thread could not start"))?;
        Ok(state)
    }

    impl PeerProofState {
        fn wait(&self) -> Result<Arc<PeerImageProof>, String> {
            let deadline = Instant::now()
                .checked_add(BOOTSTRAP_TIMEOUT)
                .ok_or_else(|| String::from("worker peer proof deadline overflow"))?;
            let mut slot = self
                .result
                .lock()
                .map_err(|_| String::from("worker peer proof state is unavailable"))?;
            loop {
                if let Some(result) = slot.as_ref() {
                    return result.clone();
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(String::from("worker peer proof timed out"));
                }
                let (next, timeout) = self
                    .ready
                    .wait_timeout(slot, remaining)
                    .map_err(|_| String::from("worker peer proof state is unavailable"))?;
                slot = next;
                if timeout.timed_out() {
                    return Err(String::from("worker peer proof timed out"));
                }
            }
        }
    }
