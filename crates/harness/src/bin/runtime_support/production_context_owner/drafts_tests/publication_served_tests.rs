// SPDX-License-Identifier: MIT

fn json_contains_key(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::Object(object) => object
            .iter()
            .any(|(key, value)| key == expected || json_contains_key(value, expected)),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_key(value, expected)),
        _ => false,
    }
}

#[test]
fn publication_replay_precedes_owner_state_cas_and_metadata_stays_identity_only() {
    let publication = create_served_publication_fixture();
    let note = "private-post-publication-note-7f13";
    let patch = ContextOwnerDraftPatchRequest {
        schema_version: CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION.to_owned(),
        request_id: "publication.draft.patch.1".to_owned(),
        draft_id: publication.request.draft_id.clone(),
        expected_version: 1,
        expected_boundary: publication.owner_binding.boundary.clone(),
        operations: vec![ContextOwnerDraftOperation::PutNote {
            note_id: "note.after-publication".to_owned(),
            text: note.to_owned(),
        }],
    };
    let (status, patched) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "edit-token",
        "PATCH",
        &run_path(&publication.run, "context-owner-drafts/publication.draft.1"),
        Some(&serde_json::to_vec(&patch).expect("draft patch JSON")),
    );
    assert_eq!(
        status, 200,
        "change live draft after publication: {patched}"
    );

    let (status, exact_replay) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &publication.publish_path,
        Some(&publication.publish_body),
    );
    assert_eq!(
        status, 200,
        "exact retry after owner-state CAS advances: {exact_replay}"
    );
    assert_eq!(exact_replay, publication.receipt_value);

    let replay_at_expiry = publication
        .fixture
        .owner
        .publish_draft_current_at(
            &publication.fixture.actor,
            &publication.fixture.snapshot,
            &publication.request,
            || Ok(publication.receipt.expires_at),
        )
        .expect("exact publication replay is recovered before the injected expiry check");
    assert_eq!(replay_at_expiry, publication.receipt);

    let changed_request = ContextOwnerDraftPublicationRequest {
        expected_draft_version: 2,
        expected_owner_state_version: publication.receipt.expected_owner_state_version + 1,
        ..publication.request.clone()
    };
    let (status, changed_replay) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &publication.publish_path,
        Some(&serde_json::to_vec(&changed_request).expect("changed publication JSON")),
    );
    assert_eq!(
        status, 409,
        "same identity with changed bytes conflicts: {changed_replay}"
    );
    assert_eq!(
        error_code(&changed_replay),
        "context_publication_request_id_reused"
    );

    let (status, view) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-published-sources"),
        None,
    );
    assert_eq!(
        status, 200,
        "current identity-only publication view: {view}"
    );
    assert_eq!(
        view["owner_state_version"],
        publication.receipt.expected_owner_state_version + 2
    );
    assert_eq!(view["publications"].as_array().unwrap().len(), 1);
    assert_eq!(
        view["publications"][0]["source_id"],
        publication.receipt.source_id
    );
    let encoded = view.to_string();
    for private_field in ["actor_subject", "request_id", "request_digest", note] {
        assert!(
            !encoded.contains(private_field),
            "metadata leaked {private_field}"
        );
    }
    assert!(
        !json_contains_key(&view, "receipt"),
        "identity metadata must not contain a publication receipt body"
    );
    assert!(
        view["binding"]["continuity"]["receipt_recovery"]
            .as_bool()
            .expect("binding receipt-recovery capability"),
        "the serialized binding may expose its receipt-recovery capability"
    );
    let catalog_after = publication
        .fixture
        .owner
        .catalog(&publication.fixture.actor)
        .expect("owner catalog after run-local publication");
    assert_eq!(catalog_after, publication.catalog_before);
    assert!(
        catalog_after.descriptors[0]
            .sources
            .iter()
            .all(|source| source.source_id != publication.receipt.source_id)
    );

    let directory = publication.fixture.directory.clone();
    drop(publication.service);
    drop(publication.fixture.owner);
    assert_storage_files_hide(&directory, note);
    assert_storage_files_hide(&directory, "trusted strategy material");
    fs::remove_dir_all(&directory).expect("remove closed isolated owner database");
}

#[test]
fn publication_adoption_link_survives_pause_resume_and_owner_reopen() {
    let publication = create_served_publication_fixture();
    let run = publication.run.clone();
    let source_id = publication.receipt.source_id.clone();
    let adoption_path = run_path(&run, &format!("context-sources/{source_id}/adopt"));

    let (status, before_stale) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-source-status"),
        None,
    );
    assert_eq!(status, 200);
    let stale_adoption = ContextSourceAdoptionRequest {
        schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
        idempotency_key: "publication.adopt.stale".to_owned(),
        expected_control_version: publication.owner_binding.boundary.control_version - 1,
        expected_revision_id: publication.owner_binding.approved_revision_id.clone(),
        expected_boundary: publication.owner_binding.boundary.clone(),
    };
    let (status, stale_result) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &adoption_path,
        Some(&serde_json::to_vec(&stale_adoption).expect("stale adoption JSON")),
    );
    assert_eq!(
        status, 409,
        "stale pre-adoption fence refuses: {stale_result}"
    );
    let (status, after_stale) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-source-status"),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(
        after_stale, before_stale,
        "stale adoption had no durable effect"
    );

    let adoption = ContextSourceAdoptionRequest {
        schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
        idempotency_key: "publication.adopt.1".to_owned(),
        expected_control_version: publication.owner_binding.boundary.control_version,
        expected_revision_id: publication.owner_binding.approved_revision_id.clone(),
        expected_boundary: publication.owner_binding.boundary.clone(),
    };
    let (status, adopted) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &adoption_path,
        Some(&serde_json::to_vec(&adoption).expect("adoption JSON")),
    );
    assert_eq!(status, 200, "explicit publication adoption: {adopted}");
    assert_eq!(adopted["command"], "commit");

    let adopted_binding = publication
        .fixture
        .owner
        .association(&publication.fixture.actor, &publication.fixture.snapshot)
        .expect("binding after publication adoption");
    let adopted_revision = adopted_binding.approved_revision_id.clone();
    let (status, active_view) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-published-sources"),
        None,
    );
    assert_eq!(
        status, 200,
        "adopted publication remains in the current view: {active_view}"
    );
    assert_eq!(active_view["active_source"]["source_id"], source_id);

    let expired_active_error = {
        let current = publication
            .fixture
            .owner
            .current
            .lock()
            .expect("owner lock");
        let entry = current.get(&run).expect("current owner entry");
        let active_revision_id = &entry.authority.state().active_revision_id;
        let (active, _) = entry
            .store
            .active_context_source(active_revision_id)
            .expect("read current active publication")
            .expect("publication is active");
        publication
            .fixture
            .owner
            .load_active_publication(
                entry,
                &publication.fixture.actor.subject,
                &active,
                publication.receipt.expires_at,
            )
            .expect_err("expired active publications are refused at the explicit boundary")
    };
    assert_eq!(
        expired_active_error.code,
        "context_publication_active_stale"
    );

    let pause = ContextControlCommand::Pause {
        idempotency_key: "publication.pause.1".to_owned(),
        expected_control_version: adopted_binding.boundary.control_version,
    };
    let (status, pause_receipt) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &run_path(&run, "context-control-commands"),
        Some(&serde_json::to_vec(&pause).expect("pause command JSON")),
    );
    assert_eq!(
        status, 200,
        "pause retains the source revision: {pause_receipt}"
    );
    let paused_binding = publication
        .fixture
        .owner
        .association(&publication.fixture.actor, &publication.fixture.snapshot)
        .expect("binding after pause");
    assert_eq!(paused_binding.approved_revision_id, adopted_revision);
    let resume = ContextControlCommand::Resume {
        idempotency_key: "publication.resume.1".to_owned(),
        expected_control_version: paused_binding.boundary.control_version,
        expected_boundary: paused_binding.boundary.clone(),
    };
    let (status, resume_receipt) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &run_path(&run, "context-control-commands"),
        Some(&serde_json::to_vec(&resume).expect("resume command JSON")),
    );
    assert_eq!(
        status, 200,
        "resume retains the source revision: {resume_receipt}"
    );
    let resumed_binding = publication
        .fixture
        .owner
        .association(&publication.fixture.actor, &publication.fixture.snapshot)
        .expect("binding after resume");
    assert_eq!(resumed_binding.approved_revision_id, adopted_revision);
    assert_ne!(resumed_binding.boundary, publication.receipt.boundary);

    let (status, replay_after_boundary_changes) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &publication.publish_path,
        Some(&publication.publish_body),
    );
    assert_eq!(
        status, 200,
        "exact replay precedes current boundary checks: {replay_after_boundary_changes}"
    );
    assert_eq!(replay_after_boundary_changes, publication.receipt_value);

    reopen_fixture_owner_store(&publication.fixture);
    let (status, recovered) = recover_served_publication(&publication);
    assert_eq!(
        status, 200,
        "publication receipt survives store reopen: {recovered}"
    );
    assert_eq!(recovered, publication.receipt_value);
    let (status, reopened_view) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&run, "context-owner-published-sources"),
        None,
    );
    assert_eq!(
        status, 200,
        "active link is authenticated after store reopen: {reopened_view}"
    );
    assert_eq!(reopened_view["active_source"]["source_id"], source_id);

    let config = ExoConfig::new(sts2_harness::EXO_SOURCE_REVISION, 64 * 1024, 1024, 1_000)
        .expect("pinned synthetic Exo config");
    let rendered = publication
        .fixture
        .owner
        .render_source_for_decision_with_config(
            &publication.fixture.actor,
            &publication.fixture.request,
            &publication.fixture.snapshot.definition_digest,
            &publication.fixture.runtime_binding,
            &publication.fixture.control_limits,
            &publication.fixture.input,
            &publication.fixture.configuration.context_ref,
            &config,
        )
        .expect("active publication resolves after pause, resume, and store reopen");
    assert_eq!(rendered.source_id, source_id);
    assert_eq!(rendered.source_version, publication.receipt.source_version);
    assert!(rendered.valid_until <= publication.receipt.expires_at);
    assert!(rendered.document.draft.notes.is_empty());
    assert_eq!(
        rendered.document.draft.draft_id,
        publication.request.draft_id
    );

    let catalog_after = publication
        .fixture
        .owner
        .catalog(&publication.fixture.actor)
        .expect("static catalog after activation");
    assert_eq!(catalog_after, publication.catalog_before);
    let directory = publication.fixture.directory.clone();
    drop(publication.service);
    drop(publication.fixture.owner);
    assert_storage_files_hide(&directory, "trusted strategy material");
    fs::remove_dir_all(&directory).expect("remove closed isolated owner database");
}
