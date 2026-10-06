// SPDX-License-Identifier: MIT

fn adopt_served_publication(publication: &ServedPublicationFixture) {
    let request = ContextSourceAdoptionRequest {
        schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
        idempotency_key: "publication.create.adopt.1".to_owned(),
        expected_control_version: publication.owner_binding.boundary.control_version,
        expected_revision_id: publication.owner_binding.approved_revision_id.clone(),
        expected_boundary: publication.owner_binding.boundary.clone(),
    };
    let (status, response) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &run_path(
            &publication.run,
            &format!("context-sources/{}/adopt", publication.receipt.source_id),
        ),
        Some(&serde_json::to_vec(&request).expect("adoption JSON")),
    );
    assert_eq!(status, 200, "served publication adoption: {response}");
}

fn create_request(
    owner: &Owner,
    actor: &AuthContext,
    snapshot: &RunSnapshot,
    draft_id: &str,
    request_id: &str,
) -> (
    ContextOwnerDraftCreateRequest,
    sts2_harness::management::ContextOwnerBinding,
) {
    let binding = owner
        .association(actor, snapshot)
        .expect("current owner binding for draft creation");
    (
        ContextOwnerDraftCreateRequest {
            schema_version: CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION.to_owned(),
            request_id: request_id.to_owned(),
            draft_id: draft_id.to_owned(),
            base_revision_id: binding.approved_revision_id.clone(),
            expected_boundary: binding.boundary.clone(),
        },
        binding,
    )
}

fn post_create(
    publication: &ServedPublicationFixture,
    request: &ContextOwnerDraftCreateRequest,
) -> (u16, serde_json::Value) {
    call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "edit-token",
        "POST",
        &run_path(&publication.run, "context-owner-drafts"),
        Some(&serde_json::to_vec(request).expect("draft create JSON")),
    )
}

fn owner_state_snapshot(owner: &Owner, workflow_run_id: &str) -> (OwnerDraftState, u64) {
    let current = owner.current.lock().expect("owner lock");
    let entry = current.get(workflow_run_id).expect("current owner entry");
    owner
        .load_draft_state(entry, workflow_run_id)
        .expect("load complete owner draft state")
}

fn owner_state_image(owner: &Owner, workflow_run_id: &str) -> (u64, Vec<u8>) {
    let (state, version) = owner_state_snapshot(owner, workflow_run_id);
    (
        version,
        state.encode().expect("encode complete owner draft state"),
    )
}

fn publication_document(publication: &ServedPublicationFixture) -> ContextSourceDocument {
    let current = publication
        .fixture
        .owner
        .current
        .lock()
        .expect("owner lock");
    let entry = current.get(&publication.run).expect("current owner entry");
    let (_, source) = entry
        .store
        .load_publication_source(
            &publication.fixture.configuration.owner_id,
            &publication.fixture.actor.subject,
            &publication.receipt.source_id,
            publication.receipt.source_version,
            &publication.receipt.source_digest,
        )
        .expect("load authenticated immutable publication")
        .expect("publication source exists");
    source.document
}

fn cleanup_publication_fixture(publication: ServedPublicationFixture) {
    let directory = publication.fixture.directory.clone();
    drop(publication.service);
    drop(publication.fixture.owner);
    fs::remove_dir_all(&directory).expect("remove isolated owner database");
}

#[test]
fn served_dynamic_publication_seeds_exact_document_and_caps_retention() {
    let mut publication = create_served_publication_fixture();
    let (configured_state, _) = owner_state_snapshot(&publication.fixture.owner, &publication.run);
    let configured = &configured_state.drafts["publication.draft.1"];
    assert_eq!(
        configured
            .base_source
            .as_ref()
            .expect("configured source base")
            .source_id,
        "strategy"
    );
    assert!(configured.envelope.retention_expires_at.is_some());
    let configured_document = publication_document(&publication);
    let excluded_reference = configured_document.draft.selected_items[0].clone();
    let (base_request, base_binding) = create_request(
        &publication.fixture.owner,
        &publication.fixture.actor,
        &publication.fixture.snapshot,
        "dynamic.source.draft",
        "dynamic.source.create",
    );
    let (status, created_source_draft) = post_create(&publication, &base_request);
    assert_eq!(
        status, 200,
        "create publication draft: {created_source_draft}"
    );
    let patch = ContextOwnerDraftPatchRequest {
        schema_version: CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION.to_owned(),
        request_id: "dynamic.source.exclude".to_owned(),
        draft_id: base_request.draft_id.clone(),
        expected_version: 1,
        expected_boundary: base_binding.boundary.clone(),
        operations: vec![ContextOwnerDraftOperation::ExcludeItem {
            reference: excluded_reference,
        }],
    };
    let (status, patched) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "edit-token",
        "PATCH",
        &run_path(
            &publication.run,
            &format!("context-owner-drafts/{}", base_request.draft_id),
        ),
        Some(&serde_json::to_vec(&patch).expect("exclude source item JSON")),
    );
    assert_eq!(
        status, 200,
        "remove source item from publication draft: {patched}"
    );
    let (status, view) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-published-sources"),
        None,
    );
    assert_eq!(status, 200, "read durable publication version: {view}");
    let publish_request = ContextOwnerDraftPublicationRequest {
        schema_version: CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "dynamic.source.publish".to_owned(),
        draft_id: base_request.draft_id.clone(),
        expected_draft_version: 2,
        expected_owner_state_version: view["owner_state_version"]
            .as_u64()
            .expect("durable owner state version"),
        expected_base_revision_id: base_binding.approved_revision_id.clone(),
        expected_binding_id: base_binding.binding_id.clone(),
        expected_binding_digest: base_binding.binding_digest.clone(),
        expected_boundary: base_binding.boundary.clone(),
    };
    let publish_path = run_path(
        &publication.run,
        &format!(
            "context-owner-drafts/{}/publications",
            base_request.draft_id
        ),
    );
    let publish_body = serde_json::to_vec(&publish_request).expect("dynamic source publish JSON");
    let (status, receipt_value) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &publish_path,
        Some(&publish_body),
    );
    assert_eq!(
        status, 200,
        "publish source with empty document: {receipt_value}"
    );
    publication.owner_binding = base_binding;
    publication.request = publish_request;
    publication.publish_path = publish_path;
    publication.publish_body = publish_body;
    publication.receipt =
        serde_json::from_value(receipt_value.clone()).expect("short-horizon publication receipt");
    publication.receipt_value = receipt_value;
    adopt_served_publication(&publication);
    let (request, binding) = create_request(
        &publication.fixture.owner,
        &publication.fixture.actor,
        &publication.fixture.snapshot,
        "dynamic.create.draft",
        "dynamic.create.request",
    );
    let expected_source = publication_document(&publication);
    assert!(expected_source.items.is_empty());
    assert!(source_valid_until(&expected_source) > publication.receipt.expires_at);
    let before = owner_state_image(&publication.fixture.owner, &publication.run);
    let (status, response) = post_create(&publication, &request);
    assert_eq!(status, 200, "served dynamic create: {response}");
    let created = receipt(&response);
    let ContextOwnerMutationResult::Draft(envelope) = &created.result else {
        panic!("dynamic create returned a non-draft receipt");
    };
    assert_eq!(envelope.binding, binding);
    let mut expected_draft = expected_source.draft;
    expected_draft.draft_id.clone_from(&request.draft_id);
    expected_draft.version = 1;
    expected_draft
        .base_revision_id
        .clone_from(&binding.approved_revision_id);
    expected_draft.author_ref = publication.fixture.actor.subject.clone();
    assert_eq!(
        serde_json::to_value(&envelope.draft).expect("created draft JSON"),
        serde_json::to_value(expected_draft).expect("source draft JSON")
    );
    let retention = envelope
        .retention_expires_at
        .expect("dynamic source has a finite publication horizon");
    assert_eq!(retention, publication.receipt.expires_at);
    let (state, version) = owner_state_snapshot(&publication.fixture.owner, &publication.run);
    assert_eq!(version, before.0 + 1);
    let stored = &state.drafts[&request.draft_id];
    assert_eq!(stored.envelope.retention_expires_at, Some(retention));
    assert_eq!(
        stored.base_source,
        Some(SourceIdentity {
            source_id: publication.receipt.source_id.clone(),
            version: publication.receipt.source_version,
            digest: publication.receipt.source_digest.clone(),
        })
    );

    let clock_called = std::cell::Cell::new(false);
    let replay = publication
        .fixture
        .owner
        .create_draft_current_with_clock(
            &publication.fixture.actor,
            &publication.fixture.snapshot,
            &request,
            || {
                clock_called.set(true);
                Ok(publication.receipt.expires_at)
            },
        )
        .expect("exact create replay precedes source-expiry lookup");
    assert!(
        !clock_called.get(),
        "exact replay must not consult expiry time"
    );
    assert_eq!(
        serde_json::to_value(replay).expect("replay receipt JSON"),
        serde_json::to_value(created).expect("original receipt JSON")
    );
    cleanup_publication_fixture(publication);
}

#[test]
fn expired_dynamic_publication_refuses_create_without_owner_state_changes() {
    // Exercise the create core at a fixed time; served link-integrity cases are separate below.
    let publication = create_served_publication_fixture();
    adopt_served_publication(&publication);
    let (request, _) = create_request(
        &publication.fixture.owner,
        &publication.fixture.actor,
        &publication.fixture.snapshot,
        "expired.create.draft",
        "expired.create.request",
    );
    let before = owner_state_image(&publication.fixture.owner, &publication.run);
    let error = publication
        .fixture
        .owner
        .create_draft_current_with_clock(
            &publication.fixture.actor,
            &publication.fixture.snapshot,
            &request,
            || Ok(publication.receipt.expires_at),
        )
        .expect_err("expired active publication must fail closed");
    assert_eq!(error.code, "context_publication_active_stale");
    assert_eq!(
        owner_state_image(&publication.fixture.owner, &publication.run),
        before,
        "expired create must not persist draft, revision, or mutation receipt"
    );
    cleanup_publication_fixture(publication);
}

#[test]
fn served_create_refuses_missing_or_discordant_adoption_links_without_effects() {
    for discordant in [false, true] {
        let publication = create_served_publication_fixture();
        adopt_served_publication(&publication);
        let database = scoped_store_path(
            &publication.fixture.configuration.store_path,
            &publication.run,
        );
        let connection = rusqlite::Connection::open(database).expect("open private fixture DB");
        let changed = if discordant {
            connection.execute(
                "UPDATE context_control_active_publication_links
                 SET active_revision_id = ?1 WHERE run_id = ?2",
                rusqlite::params!["context.revision.discordant", publication.run],
            )
        } else {
            connection.execute(
                "DELETE FROM context_control_active_publication_links WHERE run_id = ?1",
                [&publication.run],
            )
        }
        .expect("corrupt isolated active-publication link fixture");
        assert_eq!(changed, 1, "exactly one active link is modified");
        drop(connection);

        let (request, _) = create_request(
            &publication.fixture.owner,
            &publication.fixture.actor,
            &publication.fixture.snapshot,
            if discordant {
                "discordant.create.draft"
            } else {
                "missing.create.draft"
            },
            if discordant {
                "discordant.create.request"
            } else {
                "missing.create.request"
            },
        );
        let before = owner_state_image(&publication.fixture.owner, &publication.run);
        let (status, response) = post_create(&publication, &request);
        assert_ne!(
            status, 200,
            "invalid active link must be refused: {response}"
        );
        assert_eq!(
            error_code(&response),
            if discordant {
                "context_publication_active_stale"
            } else {
                "context_publication_not_found"
            }
        );
        assert_eq!(
            owner_state_image(&publication.fixture.owner, &publication.run),
            before,
            "invalid link refusal must not persist draft, revision, or receipt"
        );
        cleanup_publication_fixture(publication);
    }
}
