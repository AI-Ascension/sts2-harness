// SPDX-License-Identifier: MIT

const PUBLICATION_RUN: &str = "run.publication.storage";
const PUBLICATION_OWNER: &str = "owner.publication.storage";
const PUBLICATION_ACTOR: &str = "actor.publication.storage";
const OTHER_PUBLICATION_ACTOR: &str = "actor.publication.other";
const PUBLICATION_KEY: [u8; 32] = [0x6b; 32];

fn publication_boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: PUBLICATION_RUN.to_owned(),
        episode_id: "episode.publication".to_owned(),
        agent_id: "agent.publication".to_owned(),
        state_id: "state.publication".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "adapter.publication".to_owned(),
        model_revision: "model.publication".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

fn publication_binding(boundary: &ContextBoundary) -> ContextOwnerBinding {
    ContextOwnerBinding {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        owner_id: PUBLICATION_OWNER.to_owned(),
        owner_version: "1".to_owned(),
        invocation_id: "run.publication.storage.execution".to_owned(),
        binding_id: "binding.publication.storage".to_owned(),
        binding_version: 1,
        binding_digest: "e".repeat(64),
        context_ref: "context.publication.storage".to_owned(),
        instance_id: "instance.publication.storage".to_owned(),
        node_kind: "decide".to_owned(),
        state: ContextBindingState::Available,
        workflow_run_id: PUBLICATION_RUN.to_owned(),
        definition_digest: "f".repeat(64),
        graph_id: "graph.publication.storage".to_owned(),
        node_id: "node.publication.storage".to_owned(),
        node_execution_id: "execution.publication.storage".to_owned(),
        boundary: boundary.clone(),
        lease_epoch: 1,
        snapshot_id: "snapshot.publication.storage".to_owned(),
        approved_revision_id: "revision.publication.storage".to_owned(),
        plan_epoch: 1,
        grants: ContextBindingGrants {
            metadata_read: true,
            content_read: true,
            edit: true,
            control: true,
        },
        continuity: ContextBindingContinuity {
            survives_controller_restart: true,
            receipt_recovery: true,
            provider_session_continuity: false,
        },
    }
}

fn publication_request(
    request_id: &str,
    expected_owner_state_version: u64,
    boundary: &ContextBoundary,
) -> ContextOwnerDraftPublicationRequest {
    ContextOwnerDraftPublicationRequest {
        schema_version: CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        draft_id: format!("draft.{request_id}"),
        expected_draft_version: 1,
        expected_owner_state_version,
        expected_base_revision_id: "revision.publication.storage".to_owned(),
        expected_binding_id: "binding.publication.storage".to_owned(),
        expected_binding_digest: "e".repeat(64),
        expected_boundary: boundary.clone(),
    }
}

fn publication_store_fixture(label: &str) -> (PathBuf, ContextControlStore) {
    let directory = std::env::temp_dir().join(format!(
        "context-control-publication-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&directory).expect("create private fixture directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("restrict fixture directory");
    }
    let authority = ControlAuthority::new(publication_boundary(), "revision.publication.storage");
    let store = ContextControlStore::create(
        directory.join("control.sqlite3"),
        PUBLICATION_KEY,
        PUBLICATION_RUN,
        &authority,
        StoreMode::Enabled,
    )
    .expect("create publication test store");
    (directory, store)
}

fn publication_owner_state(store: &mut ContextControlStore) -> Vec<u8> {
    let bytes = br#"{"drafts":[],"version":1}"#;
    assert_eq!(
        store
            .compare_exchange_owner_context_state(PUBLICATION_OWNER, 0, bytes)
            .expect("seed encrypted owner state"),
        1
    );
    bytes.to_vec()
}

fn publication_source(draft_id: &str) -> DurableContextSourceSnapshot {
    let document = ContextSourceDocument {
        draft: ContextDraft::new(draft_id, "revision.publication.storage"),
        items: Default::default(),
    };
    DurableContextSourceSnapshot {
        source_id: String::new(),
        version: 1,
        digest: context_source_digest(&document).expect("publication source digest"),
        document,
    }
}

fn write_publication(
    store: &mut ContextControlStore,
    actor_subject: &str,
    request: &ContextOwnerDraftPublicationRequest,
    configured_source_count: usize,
) -> Result<DurableContextOwnerPublication, DurableControlStoreError> {
    let state = store
        .load_owner_context_state(PUBLICATION_OWNER)?
        .ok_or(DurableControlStoreError::Missing)?;
    let (source_id, request_digest) =
        store.draft_publication_identity(PUBLICATION_OWNER, actor_subject, request)?;
    let mut source = publication_source(&request.draft_id);
    source.source_id = source_id.clone();
    let receipt = ContextOwnerDraftPublicationReceipt {
        schema_version: CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION.to_owned(),
        owner_id: PUBLICATION_OWNER.to_owned(),
        workflow_run_id: PUBLICATION_RUN.to_owned(),
        actor_subject: actor_subject.to_owned(),
        binding: publication_binding(&request.expected_boundary),
        boundary: request.expected_boundary.clone(),
        request_id: request.request_id.clone(),
        request_digest,
        draft_id: request.draft_id.clone(),
        draft_version: request.expected_draft_version,
        base_revision_id: request.expected_base_revision_id.clone(),
        expected_owner_state_version: request.expected_owner_state_version,
        resulting_owner_state_version: request
            .expected_owner_state_version
            .checked_add(1)
            .ok_or(DurableControlStoreError::TooLarge)?,
        source_id,
        source_version: source.version,
        source_digest: source.digest.clone(),
        published_at: 100,
        expires_at: 200,
    };
    store.publish_draft_source(DurableContextOwnerPublicationWrite {
        owner_id: PUBLICATION_OWNER,
        actor_subject,
        request,
        receipt: &receipt,
        source: &source,
        expected_owner_state_bytes: &state.bytes,
        configured_source_count,
    })
}

fn publication_lookup(
    request: &ContextOwnerDraftPublicationRequest,
) -> ContextOwnerDraftPublicationLookupRequest {
    ContextOwnerDraftPublicationLookupRequest {
        schema_version: CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION.to_owned(),
        request: request.clone(),
    }
}

fn add_static_source(store: &mut ContextControlStore) {
    let document = ContextSourceDocument {
        draft: ContextDraft::new("configured.source", "revision.publication.storage"),
        items: Default::default(),
    };
    store
        .publish_context_source(&DurableContextSourceSnapshot {
            source_id: "configured.source.1".to_owned(),
            version: 1,
            digest: context_source_digest(&document).expect("configured source digest"),
            document,
        })
        .expect("persist one configured source fixture");
}

fn cleanup_publication_store(directory: PathBuf) {
    fs::remove_dir_all(directory).expect("remove isolated publication store");
}

fn persist_pause_control_receipt(
    store: &mut ContextControlStore,
) -> (
    ControlAuthority,
    ContextControlCommand,
    DurableContextOwnerControlReceipt,
) {
    let mut authority =
        ControlAuthority::new(publication_boundary(), "revision.publication.storage");
    let binding = publication_binding(&authority.state().boundary);
    let command = ContextControlCommand::Pause {
        idempotency_key: "migration.pause.1".to_owned(),
        expected_control_version: 1,
    };
    let outcome = authority
        .request_pause("migration.pause.1", 1)
        .expect("valid control transition");
    let state = authority.state();
    let receipt = ContextControlReceipt {
        schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.to_owned(),
        owner_id: PUBLICATION_OWNER.to_owned(),
        invocation_id: binding.invocation_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_digest: binding.binding_digest.clone(),
        command: ContextControlCommandKind::Pause,
        command_id: outcome.command_id,
        idempotency_key: outcome.idempotency_key,
        effect: outcome.effect,
        control_version: outcome.control_version,
        plan_epoch: state.plan_epoch,
        controller_epoch: state.boundary.controller_epoch,
        gate_epoch: state.boundary.gate_epoch,
        boundary: state.boundary.clone(),
        revision_id: None,
        preview_manifest_digest: None,
        approved_manifest_digest: None,
    };
    let record = DurableContextOwnerControlReceipt {
        owner_id: PUBLICATION_OWNER.to_owned(),
        actor_subject: PUBLICATION_ACTOR.to_owned(),
        binding,
        command: command.clone(),
        receipt,
    };
    store
        .persist_with_owner_control_receipt(&authority, StoreMode::Enabled, &record)
        .expect("persist authentic control receipt");
    (authority, command, record)
}
