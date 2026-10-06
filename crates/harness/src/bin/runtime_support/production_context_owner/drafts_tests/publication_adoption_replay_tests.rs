// SPDX-License-Identifier: MIT

#[test]
fn adoption_replay_is_bound_to_source_identity_and_survives_later_revision_and_reopen() {
    let publication = create_served_publication_fixture();
    let source_a = publication.receipt.source_id.clone();
    let path_a = run_path(
        &publication.run,
        &format!("context-sources/{source_a}/adopt"),
    );

    let second_publication_request = ContextOwnerDraftPublicationRequest {
        request_id: "publication.request.2".to_owned(),
        expected_owner_state_version: publication.receipt.resulting_owner_state_version,
        ..publication.request.clone()
    };
    let (status, second_publication_value) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &publication.publish_path,
        Some(
            &serde_json::to_vec(&second_publication_request)
                .expect("second publication request JSON"),
        ),
    );
    assert_eq!(
        status, 200,
        "second immutable publication: {second_publication_value}"
    );
    let second_publication: ContextOwnerDraftPublicationReceipt =
        serde_json::from_value(second_publication_value).expect("second publication receipt");
    assert_eq!(
        second_publication.source_digest, publication.receipt.source_digest,
        "both publications contain the same immutable document"
    );
    assert_ne!(second_publication.source_id, source_a);
    let path_b = run_path(
        &publication.run,
        &format!("context-sources/{}/adopt", second_publication.source_id),
    );

    let adoption = ContextSourceAdoptionRequest {
        schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
        idempotency_key: "publication.adopt.same-content".to_owned(),
        expected_control_version: publication.owner_binding.boundary.control_version,
        expected_revision_id: publication.owner_binding.approved_revision_id.clone(),
        expected_boundary: publication.owner_binding.boundary.clone(),
    };
    let adoption_body = serde_json::to_vec(&adoption).expect("adoption request JSON");
    let (status, adopted_value) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &path_a,
        Some(&adoption_body),
    );
    assert_eq!(status, 200, "adopt source A: {adopted_value}");
    let adopted_receipt: sts2_harness::management::ContextControlReceipt =
        serde_json::from_value(adopted_value.clone()).expect("adoption receipt");

    let journal_after_adopt = {
        let current = publication
            .fixture
            .owner
            .current
            .lock()
            .expect("owner lock");
        current
            .get(&publication.run)
            .expect("current owner entry")
            .authority
            .export_journal()
            .expect("export control journal")
    };
    let (status, source_status_before_conflict) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-source-status"),
        None,
    );
    assert_eq!(status, 200);
    let (status, publication_view_before_conflict) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-published-sources"),
        None,
    );
    assert_eq!(status, 200);

    let (status, wrong_source_replay) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &path_b,
        Some(&adoption_body),
    );
    assert_eq!(
        status, 409,
        "source B cannot reuse source A's receipt: {wrong_source_replay}"
    );
    assert_eq!(
        error_code(&wrong_source_replay),
        "context_source_adoption_refused"
    );
    assert!(
        wrong_source_replay
            .to_string()
            .contains("idempotency_conflict"),
        "same key with a different source has a typed conflict"
    );
    let (status, source_status_after_conflict) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-source-status"),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(source_status_after_conflict, source_status_before_conflict);
    let (status, publication_view_after_conflict) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-published-sources"),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(
        publication_view_after_conflict,
        publication_view_before_conflict
    );
    let journal_after_conflict = {
        let current = publication
            .fixture
            .owner
            .current
            .lock()
            .expect("owner lock");
        current
            .get(&publication.run)
            .expect("current owner entry")
            .authority
            .export_journal()
            .expect("export control journal")
    };
    assert_eq!(journal_after_conflict, journal_after_adopt);

    let (status, exact_adoption_replay) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &path_a,
        Some(&adoption_body),
    );
    assert_eq!(status, 200, "exact source A retry: {exact_adoption_replay}");
    assert_eq!(exact_adoption_replay, adopted_value);

    let adopted_binding = publication
        .fixture
        .owner
        .association(&publication.fixture.actor, &publication.fixture.snapshot)
        .expect("binding after adoption");
    let pause = ContextControlCommand::Pause {
        idempotency_key: "publication.adopt.pause.after".to_owned(),
        expected_control_version: adopted_binding.boundary.control_version,
    };
    let (status, pause_receipt) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &run_path(&publication.run, "context-control-commands"),
        Some(&serde_json::to_vec(&pause).expect("pause command JSON")),
    );
    assert_eq!(status, 200, "pause before ordinary commit: {pause_receipt}");
    let paused_binding = publication
        .fixture
        .owner
        .association(&publication.fixture.actor, &publication.fixture.snapshot)
        .expect("binding after pause");
    let digest = "e".repeat(64);
    let commit = ContextControlCommand::Commit {
        idempotency_key: "publication.adopt.ordinary-commit-after".to_owned(),
        expected_control_version: paused_binding.boundary.control_version,
        expected_revision_id: paused_binding.approved_revision_id.clone(),
        expected_boundary: paused_binding.boundary.clone(),
        preview_manifest_digest: digest.clone(),
        approved_manifest_digest: digest,
    };
    let (status, committed) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &run_path(&publication.run, "context-control-commands"),
        Some(&serde_json::to_vec(&commit).expect("ordinary commit JSON")),
    );
    assert_eq!(status, 200, "ordinary later revision: {committed}");
    let later_revision_id = {
        let current = publication
            .fixture
            .owner
            .current
            .lock()
            .expect("owner lock");
        current
            .get(&publication.run)
            .expect("current owner entry")
            .authority
            .state()
            .active_revision_id
            .clone()
    };
    assert_ne!(later_revision_id, adopted_binding.approved_revision_id);

    reopen_fixture_owner_store(&publication.fixture);
    let journal_before_historical_replay = {
        let current = publication
            .fixture
            .owner
            .current
            .lock()
            .expect("owner lock");
        current
            .get(&publication.run)
            .expect("reopened owner entry")
            .authority
            .export_journal()
            .expect("export reopened control journal")
    };
    let (status, source_status_before_historical_replay) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-source-status"),
        None,
    );
    assert_eq!(status, 200);
    let (status, historical_replay) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "publish-token",
        "POST",
        &path_a,
        Some(&adoption_body),
    );
    assert_eq!(
        status, 200,
        "exact historical replay after reopen: {historical_replay}"
    );
    assert_eq!(historical_replay, adopted_value);
    let journal_after_historical_replay = {
        let current = publication
            .fixture
            .owner
            .current
            .lock()
            .expect("owner lock");
        current
            .get(&publication.run)
            .expect("reopened owner entry")
            .authority
            .export_journal()
            .expect("export reopened control journal")
    };
    assert_eq!(
        journal_after_historical_replay,
        journal_before_historical_replay
    );
    let (status, source_status_after_historical_replay) = call(
        Arc::clone(&publication.service),
        Arc::clone(&publication.auth),
        "read-token",
        "GET",
        &run_path(&publication.run, "context-owner-source-status"),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(
        source_status_after_historical_replay,
        source_status_before_historical_replay
    );

    let outer = adopted_receipt;
    let inner = sts2_harness::context_control::ControlReceipt {
        command_id: outer.command_id.clone(),
        idempotency_key: outer.idempotency_key.clone(),
        effect: outer.effect.clone(),
        control_version: outer.control_version,
        plan_epoch: outer.plan_epoch,
    };
    super::publication_adoption::validate_adoption_replay_receipt(&outer, Some(&inner))
        .expect("matching durable receipt correlation is accepted");
    assert_eq!(
        super::publication_adoption::validate_adoption_replay_receipt(&outer, None)
            .expect_err("missing journal history is corruption")
            .code,
        "context_source_adoption_receipt_corrupt"
    );
    let mut discordant_inner = inner;
    discordant_inner.command_id.push_str("-different");
    assert_eq!(
        super::publication_adoption::validate_adoption_replay_receipt(
            &outer,
            Some(&discordant_inner),
        )
        .expect_err("discordant journal history is corruption")
        .code,
        "context_source_adoption_receipt_corrupt"
    );

    let directory = publication.fixture.directory.clone();
    drop(publication.service);
    drop(publication.fixture.owner);
    assert_storage_files_hide(&directory, "trusted strategy material");
    fs::remove_dir_all(&directory).expect("remove closed isolated owner database");
}
