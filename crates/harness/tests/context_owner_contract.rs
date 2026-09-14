// SPDX-License-Identifier: MIT

use std::error::Error;

use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_CATALOG_SCHEMA_VERSION,
    CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION, ContextBindingCatalog, ContextBindingContinuity,
    ContextBindingDescriptor, ContextBindingGrants, ContextBindingRequest, ContextBindingState,
    ContextControlCommand, ContextControlCommandKind, ContextControlReceipt,
    ContextEffectiveLimits, ContextOwnerBinding,
};
use sts2_harness::sha256_hex;

fn digest(seed: &str) -> String {
    sha256_hex(seed.as_bytes())
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
        control_version: binding.boundary.control_version,
        plan_epoch: binding.plan_epoch,
        controller_epoch: binding.boundary.controller_epoch,
        gate_epoch: binding.boundary.gate_epoch,
        boundary: binding.boundary.clone(),
        revision_id: None,
        preview_manifest_digest: None,
        approved_manifest_digest: None,
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
    let approved = digest("approved");
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
    );
    receipt.revision_id = Some("revision.1".to_owned());
    receipt.preview_manifest_digest = Some(preview);
    receipt.approved_manifest_digest = Some(approved);
    receipt.validate_for(&binding, &command)?;
    receipt.approved_manifest_digest = Some(digest("foreign"));
    assert!(receipt.validate_for(&binding, &command).is_err());
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
    let error = catalog
        .descriptor_for("context.live.v1", "decide")
        .err()
        .ok_or("ambiguous descriptor was accepted")?;
    assert_eq!(error.code, "context_binding_ambiguous");
    Ok(())
}
