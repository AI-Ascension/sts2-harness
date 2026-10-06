// SPDX-License-Identifier: MIT

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn owner_fixture() -> OwnerFixture {
    let directory = std::env::temp_dir().join(format!(
        "production-context-owner-drafts-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&directory).expect("create isolated owner database directory");
    #[cfg(unix)]
    {
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("restrict isolated owner database directory");
        assert_eq!(
            fs::metadata(&directory)
                .expect("inspect isolated owner database directory")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "owner fixture lives in a private Unix directory"
        );
    }

    let observation = EpisodeObservation::new(
        "combat-1",
        1,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"combat-1",
            "generation":1,
            "visible_seed":"fixture",
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
    )
    .expect("synthetic observation");
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![
            EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("legal action"),
        ],
    )
    .expect("synthetic legal-action catalog");
    let now = unix_time().expect("clock");
    let item_bytes = b"trusted strategy material".to_vec();
    let item = ContextItem {
        reference: ContextItemRef {
            item_id: "strategy.1".to_owned(),
            version: 1,
            sha256: sts2_harness::sha256_hex(&item_bytes),
        },
        kind: "strategy".to_owned(),
        bytes: item_bytes,
        protected: false,
        expires_at: now + 3_600,
    };
    let mut items = BTreeMap::new();
    items.insert(item_key(&item.reference), item.clone());
    let mut draft = ContextDraft::new("seed-draft", "context.revision.1");
    draft.selected_items.push(item.reference.clone());
    let document = ContextSourceDocument { draft, items };
    let source = ContextBindingSource {
        source_id: "strategy".to_owned(),
        version: 1,
        digest: context_source_digest(&document).expect("source digest"),
    };
    let configuration = Configuration {
        schema_version: SCHEMA.to_owned(),
        store_path: directory.join("context.sqlite3"),
        key_reference: "unused-in-test".to_owned(),
        owner_id: "production-owner".to_owned(),
        owner_version: "v1".to_owned(),
        context_ref: "context.live.v1".to_owned(),
        limits: ContextEffectiveLimits::default(),
        render_required: true,
        sources: vec![source.clone()],
        membership: None,
    };
    let owner = Arc::new(Owner {
        configuration: configuration.clone(),
        key: OWNER_KEY,
        current: Mutex::new(BTreeMap::new()),
    });
    let actor = AuthContext::new(SUBJECT, ["workflow:*".to_owned()]).expect("owner actor");
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "draft-http-run".to_owned(),
        definition: None,
        artifact_id: None,
        instance_id: "test-instance".to_owned(),
        profile: sts2_harness::management::LIVE_WORKFLOW_PROFILE.to_owned(),
        admission: None,
    };
    let definition_digest = "d".repeat(64);
    let workflow_run_id = run_id(&request, &definition_digest).expect("workflow run ID");
    let catalog = owner.catalog(&actor).expect("owner catalog");
    let descriptor = catalog.descriptors.first().expect("owner descriptor");
    let control_limits = ContextOwnerControlLimits::from_descriptors(&catalog, &[descriptor])
        .expect("admitted control limits");
    let runtime_binding = RuntimeAuthorityBinding {
        instance_id: request.instance_id.clone(),
        session_id: "session.1".to_owned(),
        lease_id: "lease.1".to_owned(),
        lease_epoch: 7,
        run_id: workflow_run_id.clone(),
        episode_id: "episode.1".to_owned(),
        trajectory_id: "trajectory.1".to_owned(),
        trace_id: "trace.1".to_owned(),
        artifact_id: "artifact.1".to_owned(),
        agent_id: "agent.1".to_owned(),
        adapter_revision: "adapter.1".to_owned(),
        model_revision: "native-model-rev-1".to_owned(),
        configuration_digest: "c".repeat(64),
        output_schema_digest: "e".repeat(64),
    };
    let observation_digest = sts2_harness::sha256_hex(
        serde_json::to_vec(observation.fair_play().as_value()).expect("observation encoding"),
    );
    let boundary = ContextBoundary {
        run_id: workflow_run_id.clone(),
        episode_id: runtime_binding.episode_id.clone(),
        agent_id: runtime_binding.agent_id.clone(),
        state_id: observation.state_id().to_owned(),
        generation: observation.generation(),
        observation_sha256: observation_digest,
        catalog_sha256: legal_catalog_digest(&actions).expect("catalog digest"),
        adapter_revision: runtime_binding.adapter_revision.clone(),
        model_revision: runtime_binding.model_revision.clone(),
        configuration_sha256: runtime_binding.configuration_digest.clone(),
        output_schema_sha256: runtime_binding.output_schema_digest.clone(),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    };
    let authority = ControlAuthority::new(boundary, "context.revision.1")
        .with_max_control_events(control_limits.max_control_events)
        .expect("selected authority event bound");
    let store_path = scoped_store_path(&configuration.store_path, &workflow_run_id);
    let store = ContextControlStore::create(
        &store_path,
        OWNER_KEY,
        &workflow_run_id,
        &authority,
        StoreMode::Enabled,
    )
    .expect("encrypted owner store");
    #[cfg(unix)]
    {
        fs::set_permissions(&store_path, fs::Permissions::from_mode(0o600))
            .expect("restrict SQLite owner-store file");
        assert_eq!(
            fs::metadata(&store_path)
                .expect("inspect SQLite owner-store file")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "SQLite owner-store file is private"
        );
    }
    owner.current.lock().expect("owner lock").insert(
        workflow_run_id.clone(),
        Current {
            authority,
            store,
            actor: actor.subject.clone(),
            definition_digest: definition_digest.clone(),
            binding_request: None,
            catalog_generation: Some(observation.generation()),
            runtime_instance_id: runtime_binding.instance_id.clone(),
            runtime_lease_id: runtime_binding.lease_id.clone(),
            runtime_lease_epoch: runtime_binding.lease_epoch,
            admitted_control_limits: control_limits.clone(),
            trusted_render: None,
        },
    );

    let bind_request = ContextBindingRequest {
        workflow_run_id: workflow_run_id.clone(),
        definition_digest: definition_digest.clone(),
        instance_id: request.instance_id.clone(),
        graph_id: "graph.1".to_owned(),
        node_id: "node.1".to_owned(),
        node_execution_id: "node-execution.1".to_owned(),
        node_kind: "decide".to_owned(),
        context_ref: configuration.context_ref.clone(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    owner
        .bind(&actor, &bind_request)
        .expect("bind current production owner");
    let snapshot = RunSnapshot {
        schema_version: RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: workflow_run_id.clone(),
        definition_digest: definition_digest.clone(),
        run_revision: 1,
        status: WorkflowRunStatus::Running,
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: bind_request.graph_id.clone(),
            node_id: bind_request.node_id.clone(),
            node_execution_id: bind_request.node_execution_id.clone(),
        },
        pending_operation: None,
        budget: Budget::default(),
        cleanup: CleanupState::NotStarted,
        admission: None,
        execution_mode: None,
    };
    let before_adoption = owner
        .source_status_current(&actor, &snapshot)
        .expect("source status before adoption");
    owner
        .publish_source_current(&actor, &snapshot, &source.source_id, &document)
        .expect("publish exact advertised source");
    owner
        .adopt_source_current(
            &actor,
            &snapshot,
            &source.source_id,
            &ContextSourceAdoptionRequest {
                schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
                idempotency_key: "adopt-source.1".to_owned(),
                expected_control_version: before_adoption.boundary.control_version,
                expected_revision_id: before_adoption.active_revision_id,
                expected_boundary: before_adoption.boundary,
            },
        )
        .expect("explicitly adopt the immutable source");

    let input = DecisionInput::new(
        ModelExecutionId::new(1).expect("execution ID"),
        observation,
        actions,
        "survive",
        Vec::new(),
    );
    let config = ExoConfig::new(sts2_harness::EXO_SOURCE_REVISION, 64 * 1024, 1024, 1_000)
        .expect("pinned synthetic Exo config");
    owner
        .render_source_for_decision_with_config(
            &actor,
            &request,
            &definition_digest,
            &runtime_binding,
            &control_limits,
            &input,
            &configuration.context_ref,
            &config,
        )
        .expect("capture trusted host-thread render snapshot with distinct model revision");

    let mut snapshot = snapshot;
    snapshot.status = WorkflowRunStatus::Running;
    let workflow_store = Arc::new(MemoryWorkflowStore::new());
    workflow_store
        .create_run(
            &request.request_id,
            &"f".repeat(64),
            snapshot.clone(),
            vec![RunEvent {
                schema_version: EVENT_SCHEMA_VERSION.to_owned(),
                workflow_run_id: workflow_run_id.clone(),
                sequence: 1,
                run_revision: snapshot.run_revision,
                event_type: EventType::RunStarted,
                definition_digest,
                node_execution_id: snapshot.cursor.node_execution_id.clone(),
                payload: EventPayload {
                    operation_id: None,
                    classification: Some(EventClassification::Accepted),
                    reason_code: "synthetic_test_fixture".to_owned(),
                },
                integrity_digest: None,
            }],
        )
        .expect("publish test run snapshot to real management store");

    OwnerFixture {
        directory,
        owner,
        configuration,
        workflow_store,
        request,
        snapshot,
        actor,
        runtime_binding,
        control_limits,
        input,
    }
}
