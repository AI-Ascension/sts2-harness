// SPDX-License-Identifier: MIT

#[test]
fn malformed_patch_http_requests_are_refused_before_owner_mutation() {
    let fixture = owner_fixture();
    let run_id = fixture.snapshot.workflow_run_id.clone();
    let owner_binding = fixture
        .owner
        .association(&fixture.actor, &fixture.snapshot)
        .expect("current production binding");
    let auth = authenticator();
    let management_service = service(
        Arc::clone(&fixture.owner),
        Arc::clone(&fixture.workflow_store),
    );
    let create = ContextOwnerDraftCreateRequest {
        schema_version: CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "patch.http.create".to_owned(),
        draft_id: "patch.http.draft".to_owned(),
        base_revision_id: owner_binding.approved_revision_id.clone(),
        expected_boundary: owner_binding.boundary.clone(),
    };
    let create_body = serde_json::to_vec(&create).expect("create request JSON");
    let (status, response) = call(
        Arc::clone(&management_service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run_id, "context-owner-drafts"),
        Some(&create_body),
    );
    assert_eq!(status, 200, "setup draft creation: {response}");

    let (status, drafts_before) = call(
        Arc::clone(&management_service),
        Arc::clone(&auth),
        "read-token",
        "GET",
        &run_path(&run_id, "context-owner-drafts"),
        None,
    );
    assert_eq!(status, 200, "read baseline draft state");

    let patch = |request_id: &str| ContextOwnerDraftPatchRequest {
        schema_version: CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        draft_id: create.draft_id.clone(),
        expected_version: 1,
        expected_boundary: owner_binding.boundary.clone(),
        operations: vec![ContextOwnerDraftOperation::SetObjective {
            text: format!("invalid request {request_id}"),
        }],
    };
    let cases = [
        (
            "patch.http.no-content-type",
            "PATCH",
            None,
            None,
            "content_type_required",
        ),
        (
            "patch.http.wrong-content-type",
            "PATCH",
            Some("text/plain"),
            None,
            "content_type_required",
        ),
        (
            "patch.http.oversized",
            "PATCH",
            Some("application/json"),
            Some(1),
            "body_too_large",
        ),
        (
            "patch.http.unsupported-method",
            "DELETE",
            Some("application/json"),
            None,
            "method_not_allowed",
        ),
    ];

    for (request_id, method, content_type, body_limit, expected_error) in cases {
        let request = patch(request_id);
        let body = serde_json::to_vec(&request).expect("patch request JSON");
        let (status, refusal) = call_with_options(
            Arc::clone(&management_service),
            Arc::clone(&auth),
            "edit-token",
            method,
            &run_path(
                &run_id,
                &format!("context-owner-drafts/{}", create.draft_id),
            ),
            Some(&body),
            HttpCallOptions {
                content_type,
                max_body_bytes: body_limit,
            },
        );
        assert_eq!(status, 400, "{method} refusal: {refusal}");
        assert_eq!(
            error_code(&refusal),
            expected_error,
            "{method} refusal code"
        );

        let lookup = ContextOwnerMutationLookupRequest {
            schema_version: CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION.to_owned(),
            request: ContextOwnerMutationRequest::PatchDraft(request),
        };
        let lookup_body = serde_json::to_vec(&lookup).expect("receipt lookup JSON");
        let (lookup_status, receipt) = call(
            Arc::clone(&management_service),
            Arc::clone(&auth),
            "read-token",
            "POST",
            &run_path(&run_id, "context-owner-mutation-receipts/lookup"),
            Some(&lookup_body),
        );
        assert_eq!(lookup_status, 200, "read refused-request receipt");
        assert!(
            receipt.is_null(),
            "a refused request must not create a receipt"
        );
    }

    let (status, drafts_after) = call(
        Arc::clone(&management_service),
        Arc::clone(&auth),
        "read-token",
        "GET",
        &run_path(&run_id, "context-owner-drafts"),
        None,
    );
    assert_eq!(status, 200, "read draft state after malformed requests");
    assert_eq!(
        drafts_after, drafts_before,
        "malformed requests changed owner state"
    );
}

#[test]
fn changed_patch_receipt_lookup_refuses_without_mutating_the_original_receipt() {
    let fixture = owner_fixture();
    let run_id = fixture.snapshot.workflow_run_id.clone();
    let owner_binding = fixture
        .owner
        .association(&fixture.actor, &fixture.snapshot)
        .expect("current production binding");
    let auth = authenticator();
    let management_service = service(
        Arc::clone(&fixture.owner),
        Arc::clone(&fixture.workflow_store),
    );
    let create = ContextOwnerDraftCreateRequest {
        schema_version: CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "patch.lookup.create".to_owned(),
        draft_id: "patch.lookup.draft".to_owned(),
        base_revision_id: owner_binding.approved_revision_id.clone(),
        expected_boundary: owner_binding.boundary.clone(),
    };
    let create_body = serde_json::to_vec(&create).expect("create request JSON");
    let (status, created) = call(
        Arc::clone(&management_service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run_id, "context-owner-drafts"),
        Some(&create_body),
    );
    assert_eq!(status, 200, "setup draft creation: {created}");

    let patch = ContextOwnerDraftPatchRequest {
        schema_version: CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION.to_owned(),
        request_id: "patch.lookup.original".to_owned(),
        draft_id: create.draft_id.clone(),
        expected_version: 1,
        expected_boundary: owner_binding.boundary.clone(),
        operations: vec![ContextOwnerDraftOperation::SetObjective {
            text: "original objective".to_owned(),
        }],
    };
    let patch_body = serde_json::to_vec(&patch).expect("patch request JSON");
    let (status, original_receipt) = client_call(
        Arc::clone(&management_service),
        Arc::clone(&auth),
        "objective-token",
        "PATCH",
        &run_path(
            &run_id,
            &format!("context-owner-drafts/{}", create.draft_id),
        ),
        Some(&patch_body),
    );
    assert_eq!(status, 200, "served patch succeeds: {original_receipt}");

    let (status, first_lookup) = lookup_mutation_receipt(
        Arc::clone(&management_service),
        Arc::clone(&auth),
        "read-token",
        &run_id,
        ContextOwnerMutationRequest::PatchDraft(patch.clone()),
    );
    assert_eq!(
        status, 200,
        "same-actor patch receipt lookup: {first_lookup}"
    );
    assert_eq!(
        first_lookup, original_receipt,
        "lookup returns the exact receipt"
    );
    let drafts_before_conflict =
        current_drafts(Arc::clone(&management_service), Arc::clone(&auth), &run_id);

    let changed_patch = ContextOwnerDraftPatchRequest {
        operations: vec![ContextOwnerDraftOperation::SetObjective {
            text: "changed objective under the original request ID".to_owned(),
        }],
        ..patch.clone()
    };
    let (status, conflict) = lookup_mutation_receipt(
        Arc::clone(&management_service),
        Arc::clone(&auth),
        "read-token",
        &run_id,
        ContextOwnerMutationRequest::PatchDraft(changed_patch),
    );
    assert_eq!(status, 409, "changed payload lookup conflicts: {conflict}");
    assert_eq!(error_code(&conflict), "context_owner_request_id_reused");
    assert_eq!(
        current_drafts(Arc::clone(&management_service), Arc::clone(&auth), &run_id),
        drafts_before_conflict,
        "changed payload lookup cannot mutate owner state"
    );

    let (status, exact_after_conflict) = lookup_mutation_receipt(
        management_service,
        auth,
        "read-token",
        &run_id,
        ContextOwnerMutationRequest::PatchDraft(patch),
    );
    assert_eq!(status, 200, "original receipt remains recoverable");
    assert_eq!(exact_after_conflict, original_receipt);
}
