// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use crate::management::{
    AuthContext, CapabilityPort, ExecutionMode, ManagementError, ManagementService,
    MemoryWorkflowStore, SeedDerivationKeyAuthority, SeedDerivationKeyReadiness, SeedKeyError,
    SeedKeyHandle, SeedModeV2, SqliteWorkflowStore, TARGET_CATALOG_SCHEMA_VERSION,
    TargetAvailability, TargetCatalogResponse, TargetDescriptor, WorkflowStore,
};

const CATALOG_REVISION: &str = "runtime-v3:seed-support-test";

struct CatalogPort {
    catalog: TargetCatalogResponse,
    calls: Arc<AtomicUsize>,
}

impl CatalogPort {
    fn new(descriptor: TargetDescriptor) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let catalog = TargetCatalogResponse {
            schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
            catalog_revision: CATALOG_REVISION.to_owned(),
            targets: vec![descriptor],
        };
        (
            Self {
                catalog,
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl CapabilityPort for CatalogPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({"capabilities": ["workflow:live", "live.workflow.v1"]}))
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.catalog.clone())
    }
}

struct ReadinessAuthority(SeedDerivationKeyReadiness);

impl SeedDerivationKeyAuthority for ReadinessAuthority {
    fn current_key(&self) -> Result<SeedKeyHandle, SeedKeyError> {
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
        self.0
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-fixtures/harness103-seed-support-service");
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

fn descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: "seed-support-owner".to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "compat.v1".to_owned(),
        capability_revision: "capabilities.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:live".to_owned()],
        capabilities: vec!["workflow:context:read".to_owned()],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: vec!["exo.runtime-v3".to_owned()],
    }
}

fn service(
    store: Arc<dyn WorkflowStore>,
    descriptor: TargetDescriptor,
    readiness: Option<SeedDerivationKeyReadiness>,
) -> ManagementService {
    let (catalog, _) = CatalogPort::new(descriptor);
    let mut service = ManagementService::new(store).with_capability_port(Arc::new(catalog));
    if let Some(readiness) = readiness {
        service =
            service.with_seed_derivation_key_authority(Arc::new(ReadinessAuthority(readiness)));
    }
    service
}

fn reader() -> AuthContext {
    AuthContext::new("seed-support-reader", ["workflow:read".to_owned()])
        .expect("read-scoped actor")
}

#[test]
fn service_catalog_separates_durable_modes_from_current_readiness() {
    let actor = reader();
    let generic = descriptor();
    let memory = ManagementService::new(Arc::new(MemoryWorkflowStore::default()))
        .with_capability_port(Arc::new(CatalogPort::new(generic.clone()).0));
    let unsupported = memory
        .target_seed_support_catalog(&actor)
        .expect("valid empty-support view");
    assert!(
        unsupported.targets[0]
            .durable_candidate_binding_modes
            .is_empty()
    );
    assert!(unsupported.targets[0].ready_candidate_modes.is_empty());
    assert_eq!(unsupported.targets[0].target, generic);
    assert_eq!(unsupported.catalog_revision, CATALOG_REVISION);

    let directory = TestDirectory::create();
    let store = Arc::new(
        SqliteWorkflowStore::open(directory.0.join("support.sqlite"))
            .expect("open durable fixture store"),
    );
    let no_authority = service(store.clone(), descriptor(), None)
        .target_seed_support_catalog(&actor)
        .expect("valid no-key support view");
    assert!(matches!(
        no_authority.targets[0]
            .durable_candidate_binding_modes
            .as_slice(),
        [SeedModeV2::Explicit]
    ));
    assert!(matches!(
        no_authority.targets[0].ready_candidate_modes.as_slice(),
        [SeedModeV2::Explicit]
    ));

    let unknown = service(
        store.clone(),
        descriptor(),
        Some(SeedDerivationKeyReadiness::Unknown),
    )
    .target_seed_support_catalog(&actor)
    .expect("configured but unknown authority remains structurally supported");
    assert!(matches!(
        unknown.targets[0]
            .durable_candidate_binding_modes
            .as_slice(),
        [SeedModeV2::Explicit, SeedModeV2::DeriveOnce]
    ));
    assert!(matches!(
        unknown.targets[0].ready_candidate_modes.as_slice(),
        [SeedModeV2::Explicit]
    ));

    let ready = service(
        store.clone(),
        descriptor(),
        Some(SeedDerivationKeyReadiness::Ready),
    )
    .target_seed_support_catalog(&actor)
    .expect("ready authority adds only a discovery hint");
    assert!(matches!(
        ready.targets[0].ready_candidate_modes.as_slice(),
        [SeedModeV2::Explicit, SeedModeV2::DeriveOnce]
    ));
    assert_eq!(unknown.support_revision, ready.support_revision);
    assert_eq!(unknown.support_revision, "seed-support.v1");
    assert_eq!(ready.catalog_revision, CATALOG_REVISION);
    assert_eq!(
        ready.targets[0].descriptor_digest,
        descriptor().digest().expect("exact V1 descriptor digest")
    );
    assert!(ready.targets[0].supported_launch_setups.is_empty());

    let mut unavailable_descriptor = descriptor();
    unavailable_descriptor.availability = TargetAvailability::Unavailable;
    let unavailable = service(
        store.clone(),
        unavailable_descriptor,
        Some(SeedDerivationKeyReadiness::Ready),
    )
    .target_seed_support_catalog(&actor)
    .expect("unavailable target keeps structural support only");
    assert!(matches!(
        unavailable.targets[0]
            .durable_candidate_binding_modes
            .as_slice(),
        [SeedModeV2::Explicit, SeedModeV2::DeriveOnce]
    ));
    assert!(unavailable.targets[0].ready_candidate_modes.is_empty());
    drop(store);
    drop(directory);
}

#[test]
fn service_requires_read_scope_before_calling_target_owner() {
    let (catalog, calls) = CatalogPort::new(descriptor());
    let service = ManagementService::new(Arc::new(MemoryWorkflowStore::default()))
        .with_capability_port(Arc::new(catalog));
    let actor = AuthContext::new("no-read-scope", Vec::<String>::new()).expect("actor");
    let error = service
        .target_seed_support_catalog(&actor)
        .err()
        .expect("target support requires read scope");
    assert_eq!(error.code, "missing_scope");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
