// SPDX-License-Identifier: MIT

fn resolve_fixture_source(fixture: &OwnerFixture) -> Result<ContextRenderSource, ManagementError> {
    fixture.owner.render_source_for_decision(
        &fixture.actor,
        &fixture.request,
        &fixture.snapshot.definition_digest,
        &fixture.runtime_binding,
        &fixture.control_limits,
        &fixture.input,
        &fixture.configuration.context_ref,
    )
}

fn pause_fixture(fixture: &OwnerFixture) -> ContextOwnerBinding {
    let binding = fixture
        .owner
        .association(&fixture.actor, &fixture.snapshot)
        .expect("current production owner binding");
    let receipt = fixture
        .owner
        .control(
            &fixture.actor,
            &binding,
            &ContextControlCommand::Pause {
                idempotency_key: "pause-render-test".to_owned(),
                expected_control_version: binding.boundary.control_version,
            },
        )
        .expect("owner authoritatively pauses its current binding");
    assert_eq!(receipt.effect, "pause_requested");
    fixture
        .owner
        .association(&fixture.actor, &fixture.snapshot)
        .expect("paused owner binding")
}

#[test]
fn paused_production_owner_captures_local_preview_but_requires_explicit_resume_for_render() {
    let fixture = owner_fixture();
    let original = resolve_fixture_source(&fixture).expect("running render source");
    let paused = pause_fixture(&fixture);
    assert_ne!(paused.boundary, original.boundary);
    let refused = resolve_fixture_source(&fixture).expect_err("pause refuses decision render");
    assert_eq!(
        refused.class,
        sts2_harness::management::ErrorClass::Conflict
    );
    assert_eq!(refused.code, sts2_harness::management::CONTEXT_OWNER_PAUSED);

    fixture
        .owner
        .create_draft_current(
            &fixture.actor,
            &fixture.snapshot,
            &ContextOwnerDraftCreateRequest {
                schema_version: CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION.to_owned(),
                request_id: "held-draft-create".to_owned(),
                draft_id: "held-draft".to_owned(),
                base_revision_id: paused.approved_revision_id.clone(),
                expected_boundary: paused.boundary.clone(),
            },
        )
        .expect("create draft against exact paused boundary");
    let preview_request = ContextOwnerPreviewRequest {
        schema_version: CONTEXT_OWNER_PREVIEW_REQUEST_SCHEMA_VERSION.to_owned(),
        request_id: "held-preview-create".to_owned(),
        draft_id: "held-draft".to_owned(),
        expected_version: 1,
        expected_boundary: paused.boundary.clone(),
    };
    let unavailable = fixture
        .owner
        .create_preview_current(&fixture.actor, &fixture.snapshot, &preview_request)
        .expect_err("a pre-pause capture cannot authorize a paused preview");
    assert_eq!(unavailable.code, "context_owner_preview_unavailable");

    let config = ExoConfig::new(sts2_harness::EXO_SOURCE_REVISION, 64 * 1024, 1024, 1_000)
        .expect("admitted provider configuration");
    let held = fixture
        .owner
        .render_source_for_decision_with_config(
            &fixture.actor,
            &fixture.request,
            &fixture.snapshot.definition_digest,
            &fixture.runtime_binding,
            &fixture.control_limits,
            &fixture.input,
            &fixture.configuration.context_ref,
            &config,
        )
        .expect_err("trusted capture does not admit provider inference while paused");
    assert_eq!(held.class, sts2_harness::management::ErrorClass::Conflict);
    assert_eq!(held.code, sts2_harness::management::CONTEXT_OWNER_PAUSED);
    let preview = fixture
        .owner
        .create_preview_current(&fixture.actor, &fixture.snapshot, &preview_request)
        .expect("held trusted input permits only an owner-local preview");
    let envelope = match &preview.result {
        ContextOwnerMutationResult::Preview(envelope) => Some(envelope),
        _ => None,
    }
    .expect("owner preview receipt");
    assert_eq!(envelope.effect_class, "owner_local_preview_only");
    assert!(envelope.blockers.is_empty());
    assert_eq!(envelope.binding, paused);
    assert_eq!(
        fixture
            .owner
            .create_preview_current(&fixture.actor, &fixture.snapshot, &preview_request,)
            .expect("exact preview receipt recovery"),
        preview
    );
    assert_eq!(
        resolve_fixture_source(&fixture)
            .expect_err("preview never resumes")
            .code,
        sts2_harness::management::CONTEXT_OWNER_PAUSED
    );

    fixture
        .owner
        .control(
            &fixture.actor,
            &paused,
            &ContextControlCommand::Resume {
                idempotency_key: "explicit-render-resume".to_owned(),
                expected_control_version: paused.boundary.control_version,
                expected_boundary: paused.boundary.clone(),
            },
        )
        .expect("explicit owner resume");
    let resumed = resolve_fixture_source(&fixture).expect("owner now admits a fresh render");
    assert_ne!(resumed.identity, original.identity);
    assert_ne!(resumed.boundary, paused.boundary);
    assert!(
        fixture
            .owner
            .assert_render_source_current(
                &fixture.actor,
                &fixture.request,
                &fixture.snapshot.definition_digest,
                &fixture.runtime_binding,
                &fixture.control_limits,
                &fixture.input,
                &fixture.configuration.context_ref,
                &original.identity,
            )
            .is_err(),
        "a pre-pause render never regains authority after resume"
    );
    let directory = fixture.directory.clone();
    drop(fixture);
    fs::remove_dir_all(directory).expect("remove closed owner database");
}

#[test]
fn pause_after_source_resolution_closes_the_last_write_fence_and_stop_is_not_temporary() {
    let fixture = owner_fixture();
    let prepared_source = resolve_fixture_source(&fixture).expect("running source resolution");
    pause_fixture(&fixture);
    let refused = fixture
        .owner
        .assert_render_source_current(
            &fixture.actor,
            &fixture.request,
            &fixture.snapshot.definition_digest,
            &fixture.runtime_binding,
            &fixture.control_limits,
            &fixture.input,
            &fixture.configuration.context_ref,
            &prepared_source.identity,
        )
        .expect_err("a pause during preparation refuses the last pre-write check");
    assert_eq!(refused.code, sts2_harness::management::CONTEXT_OWNER_PAUSED);
    fixture
        .owner
        .current
        .lock()
        .expect("owner registry")
        .get_mut(&fixture.snapshot.workflow_run_id)
        .expect("current authority")
        .authority
        .stop()
        .expect("stop the real authority");
    let stopped = resolve_fixture_source(&fixture).expect_err("stop dominates pause hold");
    assert_eq!(stopped.code, "context_render_gate_closed");
    assert_ne!(stopped.code, sts2_harness::management::CONTEXT_OWNER_PAUSED);
    let directory = fixture.directory.clone();
    drop(fixture);
    fs::remove_dir_all(directory).expect("remove closed owner database");
}
