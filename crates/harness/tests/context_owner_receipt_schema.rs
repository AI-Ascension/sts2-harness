// SPDX-License-Identifier: MIT

use std::error::Error;

use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION,
    CONTEXT_OWNER_RECEIPT_V1_SCHEMA_VERSION, ContextBindingContinuity, ContextBindingGrants,
    ContextBindingRequest, ContextBindingState, ContextControlCommand, ContextControlCommandKind,
    ContextControlReceipt, ContextOwnerBinding,
};
use sts2_harness::sha256_hex;

fn digest(seed: &str) -> String {
    sha256_hex(seed.as_bytes())
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

fn receipt(binding: &ContextOwnerBinding) -> ContextControlReceipt {
    ContextControlReceipt {
        schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.to_owned(),
        owner_id: binding.owner_id.clone(),
        invocation_id: binding.invocation_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_digest: binding.binding_digest.clone(),
        command: ContextControlCommandKind::Pause,
        command_id: "command.1".to_owned(),
        idempotency_key: "idempotency.1".to_owned(),
        effect: "pause_requested".to_owned(),
        control_version: 2,
        plan_epoch: 1,
        controller_epoch: 1,
        gate_epoch: 2,
        boundary: ContextBoundary {
            gate_epoch: 2,
            control_version: 2,
            ..binding.boundary.clone()
        },
        revision_id: None,
        preview_manifest_digest: None,
        approved_manifest_digest: None,
    }
}

#[test]
fn receipt_v1_is_rejected_without_an_unsafe_upgrade() -> Result<(), Box<dyn Error>> {
    let request = request();
    let binding = binding(&request);
    let pause = ContextControlCommand::Pause {
        idempotency_key: "idempotency.1".to_owned(),
        expected_control_version: 1,
    };
    let mut legacy = receipt(&binding);
    legacy.schema_version = CONTEXT_OWNER_RECEIPT_V1_SCHEMA_VERSION.to_owned();
    let validation_error = legacy
        .validate_for(&binding, &pause)
        .err()
        .ok_or("legacy receipt was accepted by validation")?;
    assert_eq!(
        validation_error.code,
        "context_control_receipt_schema_unsupported"
    );

    let mut legacy_json = serde_json::to_value(&legacy)?;
    let object = legacy_json
        .as_object_mut()
        .ok_or("receipt did not serialize as an object")?;
    object.remove("command");
    object.remove("boundary");
    let decode_error = serde_json::from_value::<ContextControlReceipt>(legacy_json)
        .err()
        .ok_or("legacy receipt was decoded as v2")?;
    assert!(
        decode_error
            .to_string()
            .contains("unsupported context control receipt schema")
    );
    Ok(())
}
