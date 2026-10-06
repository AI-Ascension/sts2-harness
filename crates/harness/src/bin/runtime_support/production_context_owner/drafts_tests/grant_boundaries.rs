// SPDX-License-Identifier: MIT

#[test]
fn served_owner_keeps_objective_content_metadata_and_control_grants_independent() {
    let fixture = owner_fixture();
    let owner_binding = fixture
        .owner
        .association(&fixture.actor, &fixture.snapshot)
        .expect("current production binding");
    let run = fixture.snapshot.workflow_run_id.clone();
    let auth = authenticator();
    let service = service(Arc::clone(&fixture.owner), Arc::clone(&fixture.workflow_store));
    let create = ContextOwnerDraftCreateRequest {
        schema_version: CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "independent.create.1".to_owned(),
        draft_id: "draft.independent".to_owned(),
        base_revision_id: owner_binding.approved_revision_id.clone(),
        expected_boundary: owner_binding.boundary.clone(),
    };
    let (status, body) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run, "context-owner-drafts"),
        Some(&serde_json::to_vec(&create).expect("create request JSON")),
    );
    assert_eq!(status, 200, "edit-only creation: {body}");

    let (status, read_only_edit) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "read-token",
        "POST",
        &run_path(&run, "context-owner-drafts"),
        Some(&serde_json::to_vec(&create).expect("read-only create JSON")),
    );
    assert_eq!(status, 403, "metadata read does not imply edit: {read_only_edit}");

    let objective_patch = ContextOwnerDraftPatchRequest {
        schema_version: CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION.to_owned(),
        request_id: "independent.objective.1".to_owned(),
        draft_id: create.draft_id.clone(),
        expected_version: 1,
        expected_boundary: owner_binding.boundary.clone(),
        operations: vec![ContextOwnerDraftOperation::SetObjective {
            text: "Use the safe line".to_owned(),
        }],
    };
    let note_patch = ContextOwnerDraftPatchRequest {
        schema_version: CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION.to_owned(),
        request_id: "independent.note.denied".to_owned(),
        draft_id: create.draft_id.clone(),
        expected_version: 1,
        expected_boundary: owner_binding.boundary.clone(),
        operations: vec![ContextOwnerDraftOperation::PutNote {
            note_id: "unauthorized.note".to_owned(),
            text: "must not be admitted without edit scope".to_owned(),
        }],
    };
    let (status, note_denial) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "objective-token",
        "PATCH",
        &run_path(&run, "context-owner-drafts/draft.independent"),
        Some(&serde_json::to_vec(&note_patch).expect("denied note JSON")),
    );
    assert_eq!(status, 403, "objective scope does not imply ordinary edit: {note_denial}");
    let (status, objective_result) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "objective-token",
        "PATCH",
        &run_path(&run, "context-owner-drafts/draft.independent"),
        Some(&serde_json::to_vec(&objective_patch).expect("objective patch JSON")),
    );
    assert_eq!(status, 200, "objective-only patch: {objective_result}");
    assert!(matches!(
        receipt(&objective_result).result,
        ContextOwnerMutationResult::Draft(_)
    ));

    for (token, method, suffix, body) in [
        ("objective-token", "GET", "context-owner-drafts", None),
        ("objective-token", "GET", "context-owner-items", None),
        (
            "read-token",
            "GET",
            "context-owner-items?include_content=true",
            None,
        ),
    ] {
        let (status, _) = call(
            Arc::clone(&service),
            Arc::clone(&auth),
            token,
            method,
            &run_path(&run, suffix),
            body,
        );
        assert_eq!(status, 403, "{token} must not gain {suffix} authority");
    }

    let pause = ContextControlCommand::Pause {
        idempotency_key: "independent.pause.1".to_owned(),
        expected_control_version: owner_binding.boundary.control_version,
    };
    let (status, _) = call(
        service,
        auth,
        "objective-token",
        "POST",
        &run_path(&run, "context-control-commands"),
        Some(&serde_json::to_vec(&pause).expect("pause command JSON")),
    );
    assert_eq!(status, 403, "objective authority does not imply control authority");
    let directory = fixture.directory.clone();
    drop(fixture.owner);
    fs::remove_dir_all(&directory).expect("remove closed isolated owner database");
}
