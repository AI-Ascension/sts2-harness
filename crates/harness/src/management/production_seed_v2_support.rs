// SPDX-License-Identifier: MIT

use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::{Counters, Shared, duplicate_run_tests};
use crate::management::{
    AuthContext, CapabilityPort, FileSeedDerivationKeyAuthority, LiveTargetCatalogPort,
    ManagementClient, ManagementError, ManagementServer, ManagementService, RunRequest,
    SeedDerivationKeyAuthority, SeedModeV2, SeedRequestV2, ServerConfig, SqliteWorkflowStore,
    StaticAuthenticator, TARGET_CATALOG_SCHEMA_VERSION, TargetCatalogResponse, TargetDescriptor,
    WORKFLOW_RUN_REQUEST_V2_SCHEMA, WORKFLOW_SEED_REQUEST_V2_SCHEMA, WorkflowExecutionPort,
    WorkflowRunRequestV2, WorkflowStore, live_run_id,
};

pub(super) const TOKEN: &str = "seed-v2-test-token";
pub(super) const OTHER_TOKEN: &str = "seed-v2-other-actor-token";
pub(super) const SUBJECT: &str = "duplicate-fence-actor";
const SEED_V2_TARGET_CAPABILITIES: &[&str] = &[
    "observe.fair-play.live.v1",
    "actions.catalog.v1",
    "actions.settlement.v1",
];

pub(super) struct CountingAuthority {
    inner: FileSeedDerivationKeyAuthority,
    current_reads: AtomicUsize,
    pinned_reads: AtomicUsize,
}

impl CountingAuthority {
    pub(super) fn open(path: &Path) -> Self {
        Self {
            inner: FileSeedDerivationKeyAuthority::open(path).expect("protected keyring"),
            current_reads: AtomicUsize::new(0),
            pinned_reads: AtomicUsize::new(0),
        }
    }

    pub(super) fn current_reads(&self) -> usize {
        self.current_reads.load(Ordering::SeqCst)
    }

    pub(super) fn pinned_reads(&self) -> usize {
        self.pinned_reads.load(Ordering::SeqCst)
    }
}

impl SeedDerivationKeyAuthority for CountingAuthority {
    fn current_key(
        &self,
    ) -> Result<crate::management::SeedKeyHandle, crate::management::SeedKeyError> {
        self.current_reads.fetch_add(1, Ordering::SeqCst);
        self.inner.current_key()
    }

    fn key_for(
        &self,
        authority_id: &str,
        version: &str,
    ) -> Result<Option<crate::management::SeedKeyHandle>, crate::management::SeedKeyError> {
        self.pinned_reads.fetch_add(1, Ordering::SeqCst);
        self.inner.key_for(authority_id, version)
    }
}

pub(super) struct CatalogCapabilities {
    pub(super) revision: String,
    profile: String,
    descriptor: TargetDescriptor,
    pub(super) calls: Arc<AtomicUsize>,
}

impl CapabilityPort for CatalogCapabilities {
    fn capabilities(&self) -> Result<serde_json::Value, ManagementError> {
        let mut capabilities = SEED_V2_TARGET_CAPABILITIES
            .iter()
            .map(|capability| (*capability).to_owned())
            .collect::<Vec<_>>();
        capabilities.push(self.profile.clone());
        Ok(serde_json::json!({
            "capabilities": capabilities
        }))
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(TargetCatalogResponse {
            schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
            catalog_revision: self.revision.clone(),
            targets: vec![self.descriptor.clone()],
        })
    }
}

struct SeedV2ExecutionCatalog {
    revision: String,
    descriptor: TargetDescriptor,
}

impl LiveTargetCatalogPort for SeedV2ExecutionCatalog {
    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Ok(TargetCatalogResponse {
            schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
            catalog_revision: self.revision.clone(),
            targets: vec![self.descriptor.clone()],
        })
    }
}

fn seed_v2_target_descriptor() -> TargetDescriptor {
    let mut descriptor = duplicate_run_tests::target_descriptor();
    descriptor.capabilities = SEED_V2_TARGET_CAPABILITIES
        .iter()
        .map(|capability| (*capability).to_owned())
        .collect();
    descriptor
}

fn bind_seed_v2_target_descriptor(request: &mut RunRequest, descriptor: &TargetDescriptor) {
    request
        .admission
        .as_mut()
        .expect("seed v2 fixture carries target admission")
        .descriptor_digest = descriptor
        .digest()
        .expect("seed v2 target descriptor digest");
}

pub(super) struct LiveServerFixture {
    pub(super) server: crate::management::ServerHandle,
    pub(super) runtime_counters: Shared<Counters>,
    pub(super) catalog_calls: Arc<AtomicUsize>,
}

pub(super) fn start_live_server<K>(
    store: Arc<SqliteWorkflowStore>,
    keys: Arc<K>,
    catalog_revision: &str,
) -> LiveServerFixture
where
    K: SeedDerivationKeyAuthority + 'static,
{
    start_live_server_with_catalog_revisions(store, keys, catalog_revision, catalog_revision)
}

/// Separates service catalog state from execution's pinned catalog for recovery-path fixtures.
pub(super) fn start_live_server_with_catalog_revisions<K>(
    store: Arc<SqliteWorkflowStore>,
    keys: Arc<K>,
    service_catalog_revision: &str,
    execution_catalog_revision: &str,
) -> LiveServerFixture
where
    K: SeedDerivationKeyAuthority + 'static,
{
    let (mut request, definition_digest) = duplicate_run_tests::admitted_request();
    let descriptor = seed_v2_target_descriptor();
    bind_seed_v2_target_descriptor(&mut request, &descriptor);
    request
        .admission
        .as_mut()
        .expect("seed v2 fixture carries target admission")
        .catalog_revision = execution_catalog_revision.to_owned();
    let profile = request.profile.clone();
    let run_id = live_run_id(&request, &definition_digest).expect("live run ID");
    let runtime_counters = Arc::new(Mutex::new(Counters::default()));
    let execution_catalog: Arc<dyn LiveTargetCatalogPort> = Arc::new(SeedV2ExecutionCatalog {
        revision: execution_catalog_revision.to_owned(),
        descriptor: descriptor.clone(),
    });
    let execution =
        duplicate_run_tests::live_port_with_catalog(&runtime_counters, &run_id, execution_catalog);
    let execution: Arc<dyn WorkflowExecutionPort> = Arc::new(execution);
    let catalog_calls = Arc::new(AtomicUsize::new(0));
    let store: Arc<dyn WorkflowStore> = store;
    let service = ManagementService::new(store)
        .with_definition_port(Arc::new(
            crate::management::workflow_ports::SyntheticDefinitionPort,
        ))
        .with_execution_port(execution)
        .with_capability_port(Arc::new(CatalogCapabilities {
            revision: service_catalog_revision.to_owned(),
            profile,
            descriptor,
            calls: Arc::clone(&catalog_calls),
        }))
        .with_seed_derivation_key_authority(keys);
    let actor = AuthContext::new(SUBJECT, ["workflow:*".to_owned()]).expect("test actor");
    let other_actor = AuthContext::new("other-seed-v2-actor", ["workflow:*".to_owned()])
        .expect("other test actor");
    let authenticator = StaticAuthenticator::new()
        .with_credential(TOKEN, actor)
        .expect("primary credential")
        .with_credential(OTHER_TOKEN, other_actor)
        .expect("other actor credential");
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback socket address"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::new(service)).expect("management server");
    LiveServerFixture {
        server,
        runtime_counters,
        catalog_calls,
    }
}

pub(super) fn derive_once_request() -> (WorkflowRunRequestV2, String) {
    let (mut legacy, definition_digest) = duplicate_run_tests::admitted_request();
    let descriptor = seed_v2_target_descriptor();
    bind_seed_v2_target_descriptor(&mut legacy, &descriptor);
    let request = WorkflowRunRequestV2 {
        schema_version: WORKFLOW_RUN_REQUEST_V2_SCHEMA.to_owned(),
        request_id: legacy.request_id,
        definition: legacy.definition,
        artifact_id: legacy.artifact_id,
        instance_id: legacy.instance_id,
        profile: legacy.profile,
        admission: legacy.admission,
        seed: SeedRequestV2 {
            schema_version: WORKFLOW_SEED_REQUEST_V2_SCHEMA.to_owned(),
            mode: SeedModeV2::DeriveOnce,
            seed: None,
        },
    };
    request.validate_seed().expect("valid v2 request");
    (request, definition_digest)
}

pub(super) struct PrivateDirectory {
    path: PathBuf,
}

impl PrivateDirectory {
    #[cfg(target_os = "linux")]
    pub(super) fn create() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!("sts2-seed-v2-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).expect("create private test directory");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("protect test directory");
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(target_os = "linux")]
    pub(super) fn keyring(&self, filename: &str, current: &str, keys: &[(&str, &str)]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = self.path.join(filename);
        let mut content = format!(
            "schema=ascension.seed-keyring/v1\nauthority_id=workflow-service\ncurrent_version={current}\n"
        );
        for (version, repeated_byte) in keys {
            content.push_str(&format!("key.{version}={}\n", repeated_byte.repeat(32)));
        }
        std::fs::write(&path, content).expect("write test keyring");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("protect test keyring");
        path
    }

    pub(super) fn cleanup(self) {
        std::fs::remove_dir_all(self.path).expect("close and remove test directory");
    }
}

pub(super) fn open_store(path: &Path) -> Arc<SqliteWorkflowStore> {
    Arc::new(SqliteWorkflowStore::open(path).expect("SQLite workflow store"))
}

pub(super) fn client(server: &LiveServerFixture) -> ManagementClient {
    ManagementClient::new(server.server.address(), TOKEN).expect("management client")
}

pub(super) fn request_bytes(request: &WorkflowRunRequestV2) -> Vec<u8> {
    serde_json::to_vec(request).expect("v2 request JSON")
}

pub(super) fn send_abandoned_post(server: &LiveServerFixture, body: &[u8]) -> TcpStream {
    let address = server.server.address();
    let mut stream = TcpStream::connect(address).expect("connect for abandoned response");
    let header = format!(
        "POST /v2/workflow-runs HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .expect("write request head");
    stream.write_all(body).expect("write request body");
    stream.flush().expect("flush request body");
    stream
}

pub(super) fn runtime_counts(counters: &Shared<Counters>) -> (usize, usize, usize, usize) {
    duplicate_run_tests::counts(counters)
}
