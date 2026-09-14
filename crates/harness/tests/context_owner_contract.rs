// SPDX-License-Identifier: MIT

use std::error::Error;

use sts2_harness::context_control::{ContextBoundary, ControlAuthority, ControlReceipt};
use sts2_harness::management::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_CATALOG_SCHEMA_VERSION,
    CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingRequest, ContextBindingState, ContextControlCommand,
    ContextControlCommandKind, ContextControlReceipt, ContextEffectiveLimits, ContextOwnerBinding,
};
use sts2_harness::sha256_hex;

fn digest(seed: &str) -> String {
    sha256_hex(seed.as_bytes())
}

fn control<T>(result: Result<T, String>) -> Result<T, Box<dyn Error>> {
    result.map_err(|error| std::io::Error::other(error).into())
}

fn boundary(run_id: &str) -> ContextBoundary {
    ContextBoundary {
        run_id: run_id.to_owned(),
        episode_id: "episode.1".to_owned(),
        agent_id: "agent.1".to_owned(),
        state_id: "state.1".to_owned(),
        generation: 1,
        observation_sha256: digest("observation"),
        catalog_sha256: digest("catalog"),
        adapter_revision: "adapter.v1".to_owned(),
        model_revision: "model.v1".to_owned(),
        configuration_sha256: digest("configuration"),
        output_schema_sha256: digest("schema"),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

fn request() -> ContextBindingRequest {
    ContextBindingRequest {
        workflow_run_id: "run.live.1".to_owned(),
        definition_digest: digest("definition"),
        instance_id: "instance.1".to_owned(),
        graph_id: "main".to_owned(),
        node_id: "decide".to_owned(),
        node_execution_id: "live.node.1".to_owned(),
        node_kind: "decide".to_owned(),
        context_ref: "context.live.v1".to_owned(),
        binding_id: "binding.1".to_owned(),
        binding_version: 1,
        binding_digest: digest("binding"),
    }
}

fn binding(request: &ContextBindingRequest) -> ContextOwnerBinding {
    ContextOwnerBinding {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        owner_id: "owner.1".to_owned(),
        owner_version: "owner.v1".to_owned(),
        invocation_id: "invocation.1".to_owned(),
        binding_id: request.binding_id.clone(),
        binding_version: request.binding_version,
        binding_digest: request.binding_digest.clone(),
        context_ref: request.context_ref.clone(),
        instance_id: request.instance_id.clone(),
        node_kind: request.node_kind.clone(),
        state: ContextBindingState::Available,
        workflow_run_id: request.workflow_run_id.clone(),
        definition_digest: request.definition_digest.clone(),
        graph_id: request.graph_id.clone(),
        node_id: request.node_id.clone(),
        node_execution_id: request.node_execution_id.clone(),
        boundary: boundary(&request.workflow_run_id),
        lease_epoch: 1,
        snapshot_id: "snapshot.1".to_owned(),
        approved_revision_id: "revision.1".to_owned(),
        plan_epoch: 1,
        grants: ContextBindingGrants {
            metadata_read: true,
            ..ContextBindingGrants::default()
        },
        continuity: ContextBindingContinuity {
            survives_controller_restart: true,
            receipt_recovery: true,
            provider_session_continuity: false,
        },
    }
}

fn receipt(
    binding: &ContextOwnerBinding,
    command: ContextControlCommandKind,
    effect: &str,
    control_version: u64,
    plan_epoch: u64,
    boundary: ContextBoundary,
) -> ContextControlReceipt {
    ContextControlReceipt {
        schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.to_owned(),
        owner_id: binding.owner_id.clone(),
        invocation_id: binding.invocation_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_digest: binding.binding_digest.clone(),
        command,
        command_id: "command.1".to_owned(),
        idempotency_key: "idempotency.1".to_owned(),
        effect: effect.to_owned(),
        control_version,
        plan_epoch,
        controller_epoch: boundary.controller_epoch,
        gate_epoch: boundary.gate_epoch,
        boundary,
        revision_id: None,
        preview_manifest_digest: None,
        approved_manifest_digest: None,
    }
}

fn controller_receipt(
    binding: &ContextOwnerBinding,
    command: ContextControlCommandKind,
    control: &ControlReceipt,
    boundary: ContextBoundary,
    revision_id: Option<String>,
    manifests: Option<(String, String)>,
) -> ContextControlReceipt {
    let (preview_manifest_digest, approved_manifest_digest) = manifests
        .map_or((None, None), |(preview, approved)| {
            (Some(preview), Some(approved))
        });
    ContextControlReceipt {
        schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.to_owned(),
        owner_id: binding.owner_id.clone(),
        invocation_id: binding.invocation_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_digest: binding.binding_digest.clone(),
        command,
        command_id: control.command_id.clone(),
        idempotency_key: control.idempotency_key.clone(),
        effect: control.effect.clone(),
        control_version: control.control_version,
        plan_epoch: control.plan_epoch,
        controller_epoch: boundary.controller_epoch,
        gate_epoch: boundary.gate_epoch,
        boundary,
        revision_id,
        preview_manifest_digest,
        approved_manifest_digest,
    }
}

#[test]
fn binding_request_and_binding_reject_foreign_identity() -> Result<(), Box<dyn Error>> {
    let request = request();
    request.validate()?;
    let mut binding = binding(&request);
    binding.validate_for_request(&request)?;
    binding.workflow_run_id = "foreign.run".to_owned();
    binding.boundary.run_id = "foreign.run".to_owned();
    let error = binding
        .validate_for_request(&request)
        .err()
        .ok_or("foreign binding was accepted")?;
    assert_eq!(
        error,
        sts2_harness::management::ManagementError::conflict(
            "context_owner_binding_run_mismatch",
            "context owner binding does not match the requested invocation identity",
        )
    );
    Ok(())
}

#[test]
fn receipt_binds_command_effect_and_full_boundary() -> Result<(), Box<dyn Error>> {
    let request = request();
    let binding = binding(&request);
    let pause = ContextControlCommand::Pause {
        idempotency_key: "idempotency.1".to_owned(),
        expected_control_version: 1,
    };
    let receipt = receipt(
        &binding,
        ContextControlCommandKind::Pause,
        "pause_requested",
        2,
        1,
        ContextBoundary {
            gate_epoch: 2,
            control_version: 2,
            ..binding.boundary.clone()
        },
    );
    receipt.validate_for(&binding, &pause)?;

    let commit = ContextControlCommand::Commit {
        idempotency_key: "idempotency.1".to_owned(),
        expected_control_version: 1,
        expected_revision_id: "revision.1".to_owned(),
        expected_boundary: binding.boundary.clone(),
        preview_manifest_digest: digest("preview"),
        approved_manifest_digest: digest("approved"),
    };
    assert!(receipt.validate_for(&binding, &commit).is_err());

    let mut foreign = receipt;
    foreign.boundary.agent_id = "foreign.agent".to_owned();
    assert!(foreign.validate_for(&binding, &pause).is_err());
    Ok(())
}

#[test]
fn commit_receipt_binds_revision_and_manifests() -> Result<(), Box<dyn Error>> {
    let request = request();
    let binding = binding(&request);
    let preview = digest("preview");
    let approved = preview.clone();
    let command = ContextControlCommand::Commit {
        idempotency_key: "idempotency.1".to_owned(),
        expected_control_version: 1,
        expected_revision_id: "revision.1".to_owned(),
        expected_boundary: binding.boundary.clone(),
        preview_manifest_digest: preview.clone(),
        approved_manifest_digest: approved.clone(),
    };
    let mut receipt = receipt(
        &binding,
        ContextControlCommandKind::Commit,
        "revision_committed",
        2,
        2,
        ContextBoundary {
            control_version: 2,
            ..binding.boundary.clone()
        },
    );
    receipt.revision_id = Some("revision.2".to_owned());
    receipt.preview_manifest_digest = Some(preview);
    receipt.approved_manifest_digest = Some(approved);
    receipt.validate_for(&binding, &command)?;
    receipt.approved_manifest_digest = Some(digest("foreign"));
    assert!(receipt.validate_for(&binding, &command).is_err());
    Ok(())
}

#[test]
fn controller_receipts_follow_pause_commit_and_resume_transitions() -> Result<(), Box<dyn Error>> {
    let request = request();
    let initial_binding = binding(&request);
    let mut authority = ControlAuthority::new(initial_binding.boundary.clone(), "revision.1");

    let pause_version = authority.state().control_version;
    let pause = ContextControlCommand::Pause {
        idempotency_key: "pause-controller".to_owned(),
        expected_control_version: pause_version,
    };
    let pause_control = control(authority.request_pause("pause-controller", pause_version))?;
    let pause_boundary = authority.state().boundary.clone();
    let pause_receipt = controller_receipt(
        &initial_binding,
        ContextControlCommandKind::Pause,
        &pause_control,
        pause_boundary.clone(),
        None,
        None,
    );
    pause_receipt.validate_for(&initial_binding, &pause)?;
    let mut forged_pause = pause_receipt.clone();
    forged_pause.boundary.gate_epoch += 1;
    forged_pause.gate_epoch = forged_pause.boundary.gate_epoch;
    assert_eq!(
        forged_pause
            .validate_for(&initial_binding, &pause)
            .err()
            .ok_or("foreign pause gate epoch was accepted")?
            .code,
        "context_control_receipt_transition"
    );

    let mut paused_binding = initial_binding.clone();
    paused_binding.boundary = pause_boundary.clone();
    let preview = digest("controller-manifest");
    let commit = ContextControlCommand::Commit {
        idempotency_key: "commit-controller".to_owned(),
        expected_control_version: authority.state().control_version,
        expected_revision_id: authority.state().active_revision_id.clone(),
        expected_boundary: pause_boundary,
        preview_manifest_digest: preview.clone(),
        approved_manifest_digest: preview.clone(),
    };
    let commit_control = control(authority.commit(
        "commit-controller",
        authority.state().control_version,
        "revision.1",
        &paused_binding.boundary,
        &preview,
        &preview,
    ))?;
    let commit_boundary = authority.state().boundary.clone();
    let commit_revision = authority.state().active_revision_id.clone();
    let commit_receipt = controller_receipt(
        &paused_binding,
        ContextControlCommandKind::Commit,
        &commit_control,
        commit_boundary.clone(),
        Some(commit_revision.clone()),
        Some((preview.clone(), preview)),
    );
    commit_receipt.validate_for(&paused_binding, &commit)?;
    assert_eq!(commit_receipt.plan_epoch, paused_binding.plan_epoch + 1);
    assert_ne!(commit_revision, paused_binding.approved_revision_id);

    let mut committed_binding = paused_binding;
    committed_binding.boundary = commit_boundary.clone();
    committed_binding.plan_epoch = authority.state().plan_epoch;
    committed_binding.approved_revision_id = commit_revision;
    let resume = ContextControlCommand::Resume {
        idempotency_key: "resume-controller".to_owned(),
        expected_control_version: authority.state().control_version,
        expected_boundary: commit_boundary.clone(),
    };
    let resume_control = control(authority.resume(
        "resume-controller",
        authority.state().control_version,
        &commit_boundary,
    ))?;
    let resume_receipt = controller_receipt(
        &committed_binding,
        ContextControlCommandKind::Resume,
        &resume_control,
        authority.state().boundary.clone(),
        None,
        None,
    );
    resume_receipt.validate_for(&committed_binding, &resume)?;
    assert_eq!(resume_receipt.plan_epoch, committed_binding.plan_epoch);
    Ok(())
}

#[test]
fn descriptor_and_catalog_validation_reject_ambiguous_bindings() -> Result<(), Box<dyn Error>> {
    let descriptor = ContextBindingDescriptor {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        binding_id: "binding.1".to_owned(),
        version: 1,
        digest: String::new(),
        context_ref: "context.live.v1".to_owned(),
        node_kinds: vec!["decide".to_owned()],
        sources: Vec::new(),
        operations: Vec::new(),
        effective_limits: ContextEffectiveLimits::default(),
        continuity: ContextBindingContinuity {
            survives_controller_restart: false,
            receipt_recovery: false,
            provider_session_continuity: false,
        },
        grants: ContextBindingGrants {
            metadata_read: true,
            ..ContextBindingGrants::default()
        },
        state: ContextBindingState::Available,
    }
    .seal()?;
    let mut second = descriptor.clone();
    second.binding_id = "binding.2".to_owned();
    second.digest = String::new();
    second = second.seal()?;
    let owner_id = "owner.1".to_owned();
    let owner_version = "owner.v1".to_owned();
    let descriptors = vec![descriptor, second];
    let catalog_digest = sha256_hex(serde_json::to_vec(&(
        &owner_id,
        &owner_version,
        &descriptors,
    ))?);
    let catalog = ContextBindingCatalog {
        schema_version: CONTEXT_OWNER_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id,
        owner_version,
        catalog_digest,
        descriptors,
    };
    catalog.validate()?;
    let selected = &catalog.descriptors[0];
    let mut selected_binding = binding(&request());
    selected_binding.binding_id = selected.binding_id.clone();
    selected_binding.binding_version = selected.version;
    selected_binding.binding_digest = selected.digest.clone();
    selected_binding.context_ref = selected.context_ref.clone();
    selected_binding.node_kind = "decide".to_owned();
    selected_binding.continuity = selected.continuity.clone();
    selected.validate_binding(&selected_binding)?;
    selected_binding.grants.content_read = true;
    let escalation = selected
        .validate_binding(&selected_binding)
        .err()
        .ok_or("grant escalation was accepted")?;
    assert_eq!(escalation.code, "context_owner_binding_grant_escalation");
    let error = catalog
        .descriptor_for("context.live.v1", "decide")
        .err()
        .ok_or("ambiguous descriptor was accepted")?;
    assert_eq!(error.code, "context_binding_ambiguous");
    Ok(())
}
