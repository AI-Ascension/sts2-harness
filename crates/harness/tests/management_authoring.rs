// SPDX-License-Identifier: MIT

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, DefinitionPort, DiffResult, InspectionResult, ManagementError, ManagementServer,
    ManagementService, STUDIO_SCHEMA_VERSION, ServerConfig, StaticAuthenticator,
    StudioCreateDraftRequest, StudioPublishDraftRequest, StudioSaveDraftRequest, ValidationResult,
    digest_value,
};

struct DefinitionDouble;

impl DefinitionPort for DefinitionDouble {
    fn validate(
        &self,
        definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        Ok(ValidationResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            compiler: "test-compiler.v1".to_owned(),
            diagnostics: Vec::new(),
        })
    }

    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        Ok(InspectionResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            workflow_id: None,
            workflow_version: None,
            required_capabilities: Vec::new(),
            graph_count: 0,
            node_count: 0,
        })
    }

    fn diff(
        &self,
        old_definition: &Value,
        new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        Ok(DiffResult {
            old_definition_digest: digest_value(old_definition).map_err(ManagementError::from)?,
            new_definition_digest: digest_value(new_definition).map_err(ManagementError::from)?,
            semantic_change: old_definition != new_definition,
            changed_paths: Vec::new(),
        })
    }
}

fn actor() -> Result<AuthContext, sts2_harness::management::AuthError> {
    AuthContext::new("operator", ["workflow:*".to_owned()])
}

fn document(version: &str, extra: &str) -> Value {
    json!({
        "workflow_id": "authoring-fixture",
        "version": version,
        "extra": extra,
        "graphs": []
    })
}

fn create_request(document: Value) -> StudioCreateDraftRequest {
    StudioCreateDraftRequest {
        schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
        draft_id: "draft-authoring".to_owned(),
        definition_id: "workflow-authoring".to_owned(),
        document,
        layout: json!({"nodes": [], "edges": []}),
        client_mutation_id: "mutation-create".to_owned(),
    }
}

fn service() -> ManagementService {
    ManagementService::in_memory().with_definition_port(Arc::new(DefinitionDouble))
}

#[test]
fn memory_authoring_is_revision_bound_idempotent_and_immutable_after_publish()
-> Result<(), Box<dyn std::error::Error>> {
    let service = service();
    let actor = actor()?;
    let created = service.studio_create_draft(&actor, create_request(document("1.0.0", "one")))?;
    assert_eq!(created.revision, 0);
    assert!(created.conflict.is_none());

    let duplicate =
        service.studio_create_draft(&actor, create_request(document("1.0.0", "one")))?;
    assert_eq!(duplicate, created);

    let saved = service.studio_save_draft(
        &actor,
        "draft-authoring",
        StudioSaveDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            expected_revision: created.revision,
            etag: created.etag.clone(),
            client_mutation_id: "mutation-save".to_owned(),
            document: document("1.0.1", "two"),
            layout: json!({"nodes": [{"id": "node-1"}], "edges": []}),
        },
    )?;
    assert_eq!(saved.revision, 1);

    let stale = service.studio_save_draft(
        &actor,
        "draft-authoring",
        StudioSaveDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            expected_revision: created.revision,
            etag: created.etag.clone(),
            client_mutation_id: "mutation-stale".to_owned(),
            document: document("1.0.2", "three"),
            layout: json!({"nodes": [], "edges": []}),
        },
    )?;
    assert_eq!(stale.revision, saved.revision);
    assert_eq!(
        stale.conflict.as_ref().map(|value| value.server_revision),
        Some(1)
    );

    let retried = service.studio_save_draft(
        &actor,
        "draft-authoring",
        StudioSaveDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            expected_revision: created.revision,
            etag: created.etag,
            client_mutation_id: "mutation-save".to_owned(),
            document: document("1.0.1", "two"),
            layout: json!({"nodes": [{"id": "node-1"}], "edges": []}),
        },
    )?;
    assert_eq!(retried, saved);

    let digest = digest_value(&saved.document)?;
    let published = service.studio_publish_draft(
        &actor,
        "draft-authoring",
        StudioPublishDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            expected_revision: saved.revision,
            etag: saved.etag.clone(),
            client_mutation_id: "mutation-publish".to_owned(),
            expected_definition_digest: digest.clone(),
        },
    )?;
    assert_eq!(published.outcome, "published");
    assert_eq!(
        published
            .definition
            .as_ref()
            .map(|value| value.definition_digest.as_str()),
        Some(digest.as_str())
    );

    let repeated = service.studio_publish_draft(
        &actor,
        "draft-authoring",
        StudioPublishDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            expected_revision: saved.revision,
            etag: saved.etag,
            client_mutation_id: "mutation-publish-retry".to_owned(),
            expected_definition_digest: digest,
        },
    )?;
    assert_eq!(repeated.outcome, "already_published");
    assert_eq!(service.studio_definitions(&actor)?.definitions.len(), 1);
    Ok(())
}

#[test]
fn authoring_rejects_secret_like_fields() -> Result<(), Box<dyn std::error::Error>> {
    let service = service();
    let actor = actor()?;
    let error =
        match service.studio_create_draft(&actor, create_request(json!({"api_key": "private"}))) {
            Ok(_) => return Err("secret-like authoring payload unexpectedly accepted".into()),
            Err(error) => error,
        };
    assert_eq!(error.code, "secret_like_field");
    Ok(())
}

#[test]
fn authoring_accepts_workflow_output_budget_fields() -> Result<(), Box<dyn std::error::Error>> {
    let service = service();
    let actor = actor()?;
    let created = service.studio_create_draft(
        &actor,
        create_request(json!({
            "workflow_id": "authoring-fixture",
            "limits": {"max_output_tokens": 128}
        })),
    )?;
    assert_eq!(created.revision, 0);
    Ok(())
}

#[test]
fn sqlite_authoring_reopens_drafts_and_publications() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!(
        "sts2-studio-authoring-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("workflow.sqlite3");
    let store = Arc::new(sts2_harness::management::SqliteWorkflowStore::open(&path)?);
    let service = ManagementService::new(store.clone())
        .with_authoring_store(store.clone())
        .with_definition_port(Arc::new(DefinitionDouble));
    let actor = actor()?;
    let created = service.studio_create_draft(&actor, create_request(document("2.0.0", "one")))?;
    let saved = service.studio_save_draft(
        &actor,
        &created.draft_id,
        StudioSaveDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            expected_revision: 0,
            etag: created.etag,
            client_mutation_id: "mutation-sqlite-save".to_owned(),
            document: document("2.0.1", "two"),
            layout: json!({"nodes": [], "edges": []}),
        },
    )?;
    let digest = digest_value(&saved.document)?;
    service.studio_publish_draft(
        &actor,
        &saved.draft_id,
        StudioPublishDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            expected_revision: saved.revision,
            etag: saved.etag,
            client_mutation_id: "mutation-sqlite-publish".to_owned(),
            expected_definition_digest: digest,
        },
    )?;
    drop(service);
    drop(store);

    let reopened = Arc::new(sts2_harness::management::SqliteWorkflowStore::open(&path)?);
    let restarted = ManagementService::new(reopened.clone())
        .with_authoring_store(reopened)
        .with_definition_port(Arc::new(DefinitionDouble));
    assert_eq!(
        restarted.studio_draft(&actor, "draft-authoring")?.revision,
        1
    );
    assert_eq!(restarted.studio_definitions(&actor)?.definitions.len(), 1);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn http_authoring_routes_accept_create_get_and_put() -> Result<(), Box<dyn std::error::Error>> {
    let service = Arc::new(ManagementService::in_memory());
    let authenticator = Arc::new(StaticAuthenticator::single("secret", actor()?)?);
    let config = ServerConfig::new("127.0.0.1:0".parse::<SocketAddr>()?, authenticator)?;
    let server = ManagementServer::start(config, service)?;
    let client = sts2_harness::management::ManagementClient::new(server.address(), "secret")?;

    let create = serde_json::to_vec(&create_request(document("3.0.0", "one")))?;
    let created = client.request_json("POST", "/v1/studio/drafts", Some(&create))?;
    assert_eq!(created.status, 200);
    let record: sts2_harness::management::StudioDraftRecord =
        serde_json::from_slice(&created.body)?;
    let save = serde_json::to_vec(&StudioSaveDraftRequest {
        schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
        expected_revision: record.revision,
        etag: record.etag,
        client_mutation_id: "mutation-http-save".to_owned(),
        document: document("3.0.1", "two"),
        layout: json!({"nodes": [], "edges": []}),
    })?;
    let saved = client.request_json("PUT", "/v1/studio/drafts/draft-authoring", Some(&save))?;
    assert_eq!(saved.status, 200);
    let fetched = client.request_json("GET", "/v1/studio/drafts/draft-authoring", None)?;
    assert_eq!(fetched.status, 200);
    let fetched_record: sts2_harness::management::StudioDraftRecord =
        serde_json::from_slice(&fetched.body)?;
    assert_eq!(fetched_record.revision, 1);
    let missing = client.request_json("GET", "/v1/studio/drafts/draft.missing", None)?;
    assert_eq!(missing.status, 404);
    server.shutdown()?;
    Ok(())
}
