// SPDX-License-Identifier: MIT

struct ServedPublicationFixture {
    fixture: OwnerFixture,
    service: Arc<ManagementService>,
    auth: Arc<dyn Authenticator>,
    run: String,
    owner_binding: sts2_harness::management::ContextOwnerBinding,
    catalog_before: sts2_harness::management::ContextBindingCatalog,
    request: ContextOwnerDraftPublicationRequest,
    publish_path: String,
    publish_body: Vec<u8>,
    receipt: ContextOwnerDraftPublicationReceipt,
    receipt_value: serde_json::Value,
}

fn create_served_publication_fixture() -> ServedPublicationFixture {
    let fixture = owner_fixture();
    let run = fixture.snapshot.workflow_run_id.clone();
    let auth = authenticator();
    let service = service(
        Arc::clone(&fixture.owner),
        Arc::clone(&fixture.workflow_store),
    );
    let owner_binding = fixture
        .owner
        .association(&fixture.actor, &fixture.snapshot)
        .expect("current production binding");
    let catalog_before = fixture
        .owner
        .catalog(&fixture.actor)
        .expect("owner catalog");

    let create = ContextOwnerDraftCreateRequest {
        schema_version: CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "publication.draft.create.1".to_owned(),
        draft_id: "publication.draft.1".to_owned(),
        base_revision_id: owner_binding.approved_revision_id.clone(),
        expected_boundary: owner_binding.boundary.clone(),
    };
    let (status, created) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "edit-token",
        "POST",
        &run_path(&run, "context-owner-drafts"),
        Some(&serde_json::to_vec(&create).expect("draft create JSON")),
    );
    assert_eq!(status, 200, "served draft creation: {created}");
    match &receipt(&created).result {
        ContextOwnerMutationResult::Draft(envelope) => assert_eq!(envelope.draft.version, 1),
        result => panic!("draft creation returned unexpected result: {result:?}"),
    }

    let (status, before_publication) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-published-sources"),
        None,
    );
    assert_eq!(
        status, 200,
        "initial owner publication view: {before_publication}"
    );
    assert!(
        before_publication["publications"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let owner_state_version = before_publication["owner_state_version"]
        .as_u64()
        .expect("durable owner-state version");
    assert_eq!(owner_state_version, 1);

    let request = ContextOwnerDraftPublicationRequest {
        schema_version: CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "publication.request.1".to_owned(),
        draft_id: create.draft_id.clone(),
        expected_draft_version: 1,
        expected_owner_state_version: owner_state_version,
        expected_base_revision_id: owner_binding.approved_revision_id.clone(),
        expected_binding_id: owner_binding.binding_id.clone(),
        expected_binding_digest: owner_binding.binding_digest.clone(),
        expected_boundary: owner_binding.boundary.clone(),
    };
    let publish_path = run_path(
        &run,
        &format!("context-owner-drafts/{}/publications", create.draft_id),
    );
    let publish_body = serde_json::to_vec(&request).expect("publication request JSON");
    let (status, receipt_value) = call(
        Arc::clone(&service),
        Arc::clone(&auth),
        "publish-token",
        "POST",
        &publish_path,
        Some(&publish_body),
    );
    assert_eq!(status, 200, "served immutable publication: {receipt_value}");
    let receipt: ContextOwnerDraftPublicationReceipt =
        serde_json::from_value(receipt_value.clone()).expect("publication receipt");
    assert_eq!(receipt.source_version, 1);
    assert!(receipt.source_id.starts_with("ownerpub."));
    assert!(receipt.expires_at > receipt.published_at);
    assert_eq!(receipt.expected_owner_state_version, owner_state_version);
    assert_eq!(
        receipt.resulting_owner_state_version,
        owner_state_version + 1
    );

    ServedPublicationFixture {
        fixture,
        service,
        auth,
        run,
        owner_binding,
        catalog_before,
        request,
        publish_path,
        publish_body,
        receipt,
        receipt_value,
    }
}

fn recover_served_publication(publication: &ServedPublicationFixture) -> (u16, serde_json::Value) {
    let lookup = ContextOwnerDraftPublicationLookupRequest {
        schema_version: CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION.to_owned(),
        request: publication.request.clone(),
    };
    call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "POST",
        &run_path(
            &publication.run,
            "context-owner-draft-publication-receipts/lookup",
        ),
        Some(&serde_json::to_vec(&lookup).expect("publication lookup JSON")),
    )
}

fn reopen_fixture_owner_store(fixture: &OwnerFixture) {
    let run = fixture.snapshot.workflow_run_id.clone();
    let previous = fixture
        .owner
        .current
        .lock()
        .expect("owner lock")
        .remove(&run)
        .expect("current owner entry");
    let Current {
        store: previous_store,
        actor,
        definition_digest,
        binding_request,
        runtime_instance_id,
        runtime_lease_id,
        runtime_lease_epoch,
        admitted_control_limits,
        ..
    } = previous;
    drop(previous_store);

    let store = sts2_harness::context_control::ContextControlStore::open(
        scoped_store_path(&fixture.configuration.store_path, &run),
        OWNER_KEY,
        &run,
    )
    .expect("reopen encrypted owner store");
    let authority = store.load().expect("recover durable owner authority");
    let catalog_generation = Some(authority.state().boundary.generation);
    fixture.owner.current.lock().expect("owner lock").insert(
        run,
        Current {
            authority,
            store,
            actor,
            definition_digest,
            binding_request,
            catalog_generation,
            runtime_instance_id,
            runtime_lease_id,
            runtime_lease_epoch,
            admitted_control_limits,
            trusted_render: None,
        },
    );
}
