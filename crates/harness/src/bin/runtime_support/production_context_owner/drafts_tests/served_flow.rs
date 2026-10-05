// SPDX-License-Identifier: MIT

#[test]
fn served_production_owner_allows_edit_only_preview_and_recovers_exact_receipt_after_restart() {
    let fixture = owner_fixture();
    assert_eq!(
        fixture.runtime_binding.model_revision,
        "native-model-rev-1",
        "model revision is a separate namespace from the pinned Exo source revision"
    );
    let owner_binding = fixture
        .owner
        .association(&fixture.actor, &fixture.snapshot)
        .expect("current production binding");
    let run = fixture.snapshot.workflow_run_id.clone();
    let auth = authenticator();
    let service = service(Arc::clone(&fixture.owner), Arc::clone(&fixture.workflow_store));
    let create = ContextOwnerDraftCreateRequest {
        schema_version: CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "draft.create.1".to_owned(),
        draft_id: "draft.1".to_owned(),
        base_revision_id: owner_binding.approved_revision_id.clone(),
        expected_boundary: owner_binding.boundary.clone(),
    };
    let body = serde_json::to_vec(&create).expect("create request JSON");
    let (status, original) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run, "context-owner-drafts"),
        Some(&body),
    );
    assert_eq!(status, 200, "edit-only draft creation: {original}");
    let original_receipt = receipt(&original);
    assert!(matches!(original_receipt.result, ContextOwnerMutationResult::Draft(_)));

    let (status, duplicate) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run, "context-owner-drafts"),
        Some(&body),
    );
    assert_eq!(status, 200, "exact retry: {duplicate}");
    assert_eq!(duplicate, original, "exact request returns its immutable receipt");

    let changed = ContextOwnerDraftCreateRequest {
        draft_id: "draft.changed".to_owned(),
        ..create.clone()
    };
    let (status, request_conflict) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run, "context-owner-drafts"),
        Some(&serde_json::to_vec(&changed).expect("changed create request JSON")),
    );
    assert_eq!(status, 409, "request ID reuse with another payload conflicts: {request_conflict}");

    let note_text = "owner-note-confidential-sentinel-4bd7";
    let note_patch = ContextOwnerDraftPatchRequest {
        schema_version: CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION.to_owned(),
        request_id: "draft.note.1".to_owned(),
        draft_id: "draft.1".to_owned(),
        expected_version: 1,
        expected_boundary: owner_binding.boundary.clone(),
        operations: vec![ContextOwnerDraftOperation::PutNote {
            note_id: "note.1".to_owned(),
            text: note_text.to_owned(),
        }],
    };
    let (status, note_result) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "edit-token",
        "PATCH",
        &run_path(&run, "context-owner-drafts/draft.1"),
        Some(&serde_json::to_vec(&note_patch).expect("note patch JSON")),
    );
    assert_eq!(status, 200, "finite active-source note authoring: {note_result}");
    assert!(matches!(
        receipt(&note_result).result,
        ContextOwnerMutationResult::Draft(_)
    ));

    let preview_request = ContextOwnerPreviewRequest {
        schema_version: CONTEXT_OWNER_PREVIEW_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "draft.preview.1".to_owned(),
        draft_id: "draft.1".to_owned(),
        expected_version: 2,
        expected_boundary: owner_binding.boundary.clone(),
    };
    let (status, preview_value) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run, "context-owner-drafts/draft.1/previews"),
        Some(&serde_json::to_vec(&preview_request).expect("preview request JSON")),
    );
    assert_eq!(status, 200, "edit-only preview: {preview_value}");
    assert!(matches!(
        receipt(&preview_value).result,
        ContextOwnerMutationResult::Preview(_)
    ));
    let serialized_preview = preview_value.to_string();
    assert!(!serialized_preview.contains("trusted strategy material"));
    assert!(!serialized_preview.contains(note_text));
    assert!(!serialized_preview.contains("prepared_bytes"));

    let (status, authored_content) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "content-token",
        "GET",
        &run_path(
            &run,
            "context-owner-items?draft_id=draft.1&include_content=true",
        ),
        None,
    );
    assert_eq!(status, 200, "authorized exact draft item bytes: {authored_content}");
    let items = authored_content["items"].as_array().expect("item array");
    let note = items
        .iter()
        .find(|item| item["reference"]["item_id"] == "draft-note.draft.1.note.1")
        .expect("draft note appears in its owner-resolved eligible set");
    let content = note["content"].as_array().expect("content projection bytes");
    let content = content
        .iter()
        .map(|byte| {
            byte.as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .expect("content byte")
        })
        .collect::<Vec<_>>();
    assert_eq!(content, note_text.as_bytes());

    let (status, metadata) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-items?draft_id=draft.1"),
        None,
    );
    assert_eq!(status, 200, "metadata-only draft item projection: {metadata}");
    assert!(!metadata.to_string().contains(note_text));
    let metadata_note = metadata["items"]
        .as_array()
        .expect("metadata item array")
        .iter()
        .find(|item| item["reference"]["item_id"] == "draft-note.draft.1.note.1")
        .expect("metadata includes the authorized note reference");
    assert!(metadata_note.get("content").is_none());
    assert_storage_files_hide(&fixture.directory, note_text);

    let (status, drafts_before_foreign) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-drafts"),
        None,
    );
    assert_eq!(status, 200, "read owner drafts before foreign attempt");
    let (status, foreign_error) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "foreign-edit-token",
        "POST",
        &run_path(&run, "context-owner-drafts"),
        Some(&serde_json::to_vec(&ContextOwnerDraftCreateRequest {
            request_id: "foreign.create.1".to_owned(),
            draft_id: "draft.foreign".to_owned(),
            ..create.clone()
        })
        .expect("foreign request JSON")),
    );
    assert_eq!(status, 403, "foreign subject is refused: {foreign_error}");
    assert_eq!(error_code(&foreign_error), "context_owner_actor");
    let (status, drafts_after_foreign) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-drafts"),
        None,
    );
    assert_eq!(status, 200, "read owner drafts after foreign attempt");
    assert_eq!(drafts_after_foreign, drafts_before_foreign, "foreign actor wrote no draft revision");
    let (status, absent_receipt) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "read-token",
        "POST",
        &run_path(&run, "context-owner-mutation-receipts/lookup"),
        Some(
            &serde_json::to_vec(&ContextOwnerMutationLookupRequest {
                schema_version: CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION.to_owned(),
                request: ContextOwnerMutationRequest::CreateDraft(
                    ContextOwnerDraftCreateRequest {
                        request_id: "foreign.create.1".to_owned(),
                        draft_id: "draft.foreign".to_owned(),
                        ..create.clone()
                    },
                ),
            })
            .expect("foreign lookup JSON"),
        ),
    );
    assert_eq!(status, 200);
    assert!(absent_receipt.is_null(), "foreign attempt wrote no terminal receipt");

    let lookup = ContextOwnerMutationLookupRequest {
        schema_version: CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION.to_owned(),
        request: ContextOwnerMutationRequest::CreateDraft(create.clone()),
    };
    let restart_owner = Arc::new(Owner {
        configuration: fixture.configuration.clone(),
        key: OWNER_KEY,
        current: Mutex::new(BTreeMap::new()),
    });
    let restarted_service = service(restart_owner, Arc::clone(&fixture.workflow_store));
    let (status, recovered) = call(
        restarted_service,
        auth,
        "read-token",
        "POST",
        &run_path(&run, "context-owner-mutation-receipts/lookup"),
        Some(&serde_json::to_vec(&lookup).expect("lookup request JSON")),
    );
    assert_eq!(status, 200, "historical same-actor receipt recovery: {recovered}");
    assert_eq!(
        receipt(&recovered),
        original_receipt,
        "restart recovery returns the original exact stored receipt without current association"
    );
    let directory = fixture.directory.clone();
    drop(service);
    drop(fixture.owner);
    fs::remove_dir_all(&directory).expect("remove closed isolated owner database");
}
