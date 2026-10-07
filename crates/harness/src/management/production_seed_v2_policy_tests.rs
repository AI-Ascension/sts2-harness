// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::super::duplicate_run_tests;
use super::super::seed_v2_support::derive_once_request;
use crate::management::{
    AuthContext, CapabilityPort, CommandApplication, CommandContext, ManagementError,
    ManagementService, RunAdmission, RunRequest, RunReservation, SeedDerivationKeyAuthority,
    SeedDerivationKeyReadiness, SeedKeyError, SeedKeyHandle, SqliteWorkflowStore,
    TARGET_CATALOG_SCHEMA_VERSION, TargetCatalogResponse, TargetDescriptor, WorkflowExecutionPort,
    WorkflowStore,
};

struct CatalogPort {
    catalog: TargetCatalogResponse,
    calls: Arc<AtomicUsize>,
}

impl CapabilityPort for CatalogPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({
            "capabilities": [
                "live.workflow.v1",
                "observe.fair-play.live.v1",
                "actions.catalog.v1",
                "actions.settlement.v1"
            ]
        }))
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.catalog.clone())
    }
}

#[derive(Default)]
struct ExecutionCalls(AtomicUsize);

impl WorkflowExecutionPort for ExecutionCalls {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ManagementError::unavailable(
            "unexpected_execution",
            "execution spy must not be reached",
        ))
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ManagementError::unavailable(
            "unexpected_execution",
            "execution spy must not be reached",
        ))
    }

    fn prepare_seed_candidate_with_reservation(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
        _admission: Option<&crate::management::TargetAdmissionBinding>,
        _reserve: &RunReservation,
    ) -> Result<RunAdmission, ManagementError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ManagementError::unavailable(
            "unexpected_execution",
            "execution spy must not be reached",
        ))
    }
}

struct UnknownKeyAuthority(Arc<AtomicUsize>);

impl SeedDerivationKeyAuthority for UnknownKeyAuthority {
    fn current_key(&self) -> Result<SeedKeyHandle, SeedKeyError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(SeedKeyError::Unavailable)
    }

    fn key_for(
        &self,
        _authority_id: &str,
        _version: &str,
    ) -> Result<Option<SeedKeyHandle>, SeedKeyError> {
        Ok(None)
    }

    fn readiness(&self) -> SeedDerivationKeyReadiness {
        SeedDerivationKeyReadiness::Unknown
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-fixtures/harness103-seed-policy");
        fs::create_dir_all(&root).expect("create designated test fixture root");
        let path = root.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated test directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn target_descriptor() -> TargetDescriptor {
    let mut descriptor = duplicate_run_tests::target_descriptor();
    descriptor.capabilities = [
        "observe.fair-play.live.v1",
        "actions.catalog.v1",
        "actions.settlement.v1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    descriptor
}

fn row_count(store: &SqliteWorkflowStore, table: &str) -> i64 {
    assert!(matches!(
        table,
        "management_seed_operations"
            | "management_submissions"
            | "management_runs"
            | "management_events"
            | "management_seed_bindings"
    ));
    store
        .connection
        .lock()
        .expect("SQLite connection lock")
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("count workflow rows")
}

#[test]
fn derive_once_missing_without_configured_authority_writes_nothing_and_never_executes() {
    let directory = TestDirectory::create();
    let store = Arc::new(
        SqliteWorkflowStore::open(directory.0.join("missing-authority.sqlite"))
            .expect("open durable workflow store"),
    );
    let (mut request, _) = derive_once_request();
    let binding = request
        .admission
        .as_mut()
        .expect("seed request carries admission");
    let descriptor = target_descriptor();
    binding.descriptor_digest = descriptor.digest().expect("descriptor digest");
    let catalog = TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: binding.catalog_revision.clone(),
        targets: vec![descriptor],
    };
    let catalog_calls = Arc::new(AtomicUsize::new(0));
    let target_port = Arc::new(CatalogPort {
        catalog,
        calls: Arc::clone(&catalog_calls),
    });
    let execution = Arc::new(ExecutionCalls::default());
    let workflow_store: Arc<dyn WorkflowStore> = store.clone();
    let service = ManagementService::new(workflow_store)
        .with_definition_port(Arc::new(
            crate::management::workflow_ports::SyntheticDefinitionPort,
        ))
        .with_capability_port(target_port)
        .with_execution_port(execution.clone());
    let actor = AuthContext::new(
        "duplicate-fence-actor",
        ["workflow:control".to_owned(), "workflow:live".to_owned()],
    )
    .expect("authorized live actor");

    let error = service
        .submit_seeded_run_v2(&actor, request)
        .err()
        .expect("new derive-once request requires configured key authority");
    assert_eq!(error.code, "seed_candidate_support_unavailable");
    assert_eq!(catalog_calls.load(Ordering::SeqCst), 1);
    assert_eq!(execution.0.load(Ordering::SeqCst), 0);
    for table in [
        "management_seed_operations",
        "management_submissions",
        "management_runs",
        "management_events",
        "management_seed_bindings",
    ] {
        assert_eq!(row_count(&store, table), 0, "no new rows in {table}");
    }

    let (mut unknown_request, _) = derive_once_request();
    let unknown_binding = unknown_request
        .admission
        .as_mut()
        .expect("second seed request carries admission");
    let descriptor = target_descriptor();
    unknown_binding.descriptor_digest = descriptor.digest().expect("descriptor digest");
    let unknown_catalog = TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: unknown_binding.catalog_revision.clone(),
        targets: vec![descriptor],
    };
    let unknown_target = Arc::new(CatalogPort {
        catalog: unknown_catalog,
        calls: Arc::clone(&catalog_calls),
    });
    let current_key_reads = Arc::new(AtomicUsize::new(0));
    let unknown_authority = Arc::new(UnknownKeyAuthority(Arc::clone(&current_key_reads)));
    let unknown_execution = Arc::new(ExecutionCalls::default());
    let workflow_store: Arc<dyn WorkflowStore> = store.clone();
    let unknown_service = ManagementService::new(workflow_store)
        .with_definition_port(Arc::new(
            crate::management::workflow_ports::SyntheticDefinitionPort,
        ))
        .with_capability_port(unknown_target)
        .with_execution_port(unknown_execution.clone())
        .with_seed_derivation_key_authority(unknown_authority);
    let error = unknown_service
        .submit_seeded_run_v2(&actor, unknown_request)
        .err()
        .expect("unknown readiness proceeds to fail-closed current-key retrieval");
    assert_eq!(error.code, "seed_key_authority_unavailable");
    assert_eq!(current_key_reads.load(Ordering::SeqCst), 1);
    assert_eq!(unknown_execution.0.load(Ordering::SeqCst), 0);
    assert_eq!(catalog_calls.load(Ordering::SeqCst), 2);
    for table in [
        "management_seed_operations",
        "management_submissions",
        "management_runs",
        "management_events",
        "management_seed_bindings",
    ] {
        assert_eq!(row_count(&store, table), 0, "no new rows in {table}");
    }
    drop(unknown_service);
    drop(service);
    drop(store);
    drop(directory);
}
