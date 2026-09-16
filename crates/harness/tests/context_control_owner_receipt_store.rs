// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use sts2_harness::context_control::{
    ContextBoundary, ContextControlStore, ControlAuthority, DurableContextOwnerControlReceipt,
    DurableControlStoreError, DurableStoreFailpoint, StoreMode,
};
use sts2_harness::management::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION,
    ContextBindingContinuity, ContextBindingGrants, ContextBindingState, ContextControlCommand,
    ContextControlCommandKind, ContextControlReceipt, ContextOwnerBinding,
};

const RUN_ID: &str = "run.receipt.storage.1";
const OWNER_ID: &str = "owner.receipt.storage";
const ACTOR: &str = "operator.receipt.storage";
const KEY: [u8; 32] = [0x51; 32];

fn boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: RUN_ID.into(),
        episode_id: "episode.1".into(),
        agent_id: "agent.1".into(),
        state_id: "state.1".into(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "adapter.1".into(),
        model_revision: "model.1".into(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

fn binding() -> ContextOwnerBinding {
    ContextOwnerBinding {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.into(),
        owner_id: OWNER_ID.into(),
        owner_version: "1".into(),
        invocation_id: "run.receipt.storage.1.execution.1".into(),
        binding_id: "binding.1".into(),
        binding_version: 1,
        binding_digest: "e".repeat(64),
        context_ref: "context.1".into(),
        instance_id: "instance.1".into(),
        node_kind: "decide".into(),
        state: ContextBindingState::Available,
        workflow_run_id: RUN_ID.into(),
        definition_digest: "f".repeat(64),
        graph_id: "graph.1".into(),
        node_id: "node.1".into(),
        node_execution_id: "execution.1".into(),
        boundary: boundary(),
        lease_epoch: 7,
        snapshot_id: "snapshot.1".into(),
        approved_revision_id: "revision.1".into(),
        plan_epoch: 1,
        grants: ContextBindingGrants {
            metadata_read: true,
            content_read: false,
            edit: false,
            control: true,
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
    command: &ContextControlCommand,
    outcome: sts2_harness::ControlReceipt,
    authority: &ControlAuthority,
) -> ContextControlReceipt {
    let state = authority.state();
    let (kind, commit_identity) = match command {
        ContextControlCommand::Pause { .. } => (ContextControlCommandKind::Pause, None),
        ContextControlCommand::Resume { .. } => (ContextControlCommandKind::Resume, None),
        ContextControlCommand::Commit {
            expected_revision_id,
            preview_manifest_digest,
            approved_manifest_digest,
            ..
        } => (
            ContextControlCommandKind::Commit,
            Some((
                expected_revision_id.clone(),
                preview_manifest_digest.clone(),
                approved_manifest_digest.clone(),
            )),
        ),
    };
    let (revision_id, preview_manifest_digest, approved_manifest_digest) = commit_identity
        .map_or((None, None, None), |(revision, preview, approved)| {
            (Some(revision), Some(preview), Some(approved))
        });
    ContextControlReceipt {
        schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.into(),
        owner_id: binding.owner_id.clone(),
        invocation_id: binding.invocation_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_digest: binding.binding_digest.clone(),
        command: kind,
        command_id: outcome.command_id,
        idempotency_key: outcome.idempotency_key,
        effect: outcome.effect,
        control_version: outcome.control_version,
        plan_epoch: outcome.plan_epoch,
        controller_epoch: state.boundary.controller_epoch,
        gate_epoch: state.boundary.gate_epoch,
        boundary: state.boundary.clone(),
        revision_id,
        preview_manifest_digest,
        approved_manifest_digest,
    }
}

fn record(
    binding: ContextOwnerBinding,
    command: ContextControlCommand,
    receipt: ContextControlReceipt,
) -> DurableContextOwnerControlReceipt {
    DurableContextOwnerControlReceipt {
        owner_id: OWNER_ID.into(),
        actor_subject: ACTOR.into(),
        binding,
        command,
        receipt,
    }
}

fn store_path() -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "context-owner-receipt-store-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&directory).expect("create fixture directory");
    directory.join("control.sqlite3")
}

#[test]
fn exact_encrypted_receipt_is_atomic_and_read_does_not_claim_the_live_writer() {
    let path = store_path();
    let initial_binding = binding();
    let initial = ControlAuthority::new(
        initial_binding.boundary.clone(),
        initial_binding.approved_revision_id.clone(),
    );
    let mut live_store =
        ContextControlStore::create(&path, KEY, RUN_ID, &initial, StoreMode::Enabled)
            .expect("create authority store");

    let mut paused = initial.clone();
    let pause = ContextControlCommand::Pause {
        idempotency_key: "same-key".into(),
        expected_control_version: initial_binding.boundary.control_version,
    };
    let outcome = paused
        .request_pause("same-key", initial_binding.boundary.control_version)
        .expect("pause accepted");
    let pause_receipt = receipt(&initial_binding, &pause, outcome, &paused);
    let pause_record = record(initial_binding.clone(), pause.clone(), pause_receipt);
    live_store
        .persist_with_owner_control_receipt(&paused, StoreMode::Enabled, &pause_record)
        .expect("atomically persist accepted command and receipt");

    // Opening and reading a receipt does not claim or evict the existing writer.
    let historical_reader =
        ContextControlStore::open(&path, KEY, RUN_ID).expect("open read handle");
    assert_eq!(
        historical_reader
            .lookup_owner_control_receipt(OWNER_ID, ACTOR, &pause)
            .expect("exact lookup"),
        Some(pause_record.clone())
    );
    live_store
        .persist(&paused, StoreMode::Enabled)
        .expect("read handle did not fence live writer");

    // A failed transaction leaves neither a new authority state nor an orphan receipt.
    let mut resumed = paused.clone();
    let resume_binding = ContextOwnerBinding {
        boundary: resumed.state().boundary.clone(),
        ..initial_binding.clone()
    };
    let failed_command = ContextControlCommand::Resume {
        idempotency_key: "failed-resume".into(),
        expected_control_version: resume_binding.boundary.control_version,
        expected_boundary: resume_binding.boundary.clone(),
    };
    let failed_outcome = resumed
        .resume(
            "failed-resume",
            resume_binding.boundary.control_version,
            &resume_binding.boundary,
        )
        .expect("resume state transition");
    let failed_receipt = receipt(&resume_binding, &failed_command, failed_outcome, &resumed);
    let failed_record = record(resume_binding, failed_command.clone(), failed_receipt);
    live_store.set_failpoint(Some(DurableStoreFailpoint::BeforeCommit));
    assert_eq!(
        live_store.persist_with_owner_control_receipt(&resumed, StoreMode::Enabled, &failed_record),
        Err(DurableControlStoreError::Failpoint)
    );
    assert_eq!(
        historical_reader
            .lookup_owner_control_receipt(OWNER_ID, ACTOR, &failed_command)
            .expect("lookup after rollback"),
        None
    );

    // A distinct command cannot reuse the same idempotency key, even if its
    // transition would otherwise be valid under a separately built authority.
    let mut alternate = ControlAuthority::new(
        initial_binding.boundary.clone(),
        initial_binding.approved_revision_id.clone(),
    );
    alternate
        .request_pause("seed-pause", initial_binding.boundary.control_version)
        .expect("seed pause");
    let alternate_binding = ContextOwnerBinding {
        boundary: alternate.state().boundary.clone(),
        ..initial_binding
    };
    let conflict_command = ContextControlCommand::Resume {
        idempotency_key: "same-key".into(),
        expected_control_version: alternate_binding.boundary.control_version,
        expected_boundary: alternate_binding.boundary.clone(),
    };
    let alternate_outcome = alternate
        .resume(
            "same-key",
            alternate_binding.boundary.control_version,
            &alternate_binding.boundary,
        )
        .expect("alternate resume transition");
    let conflict_receipt = receipt(
        &alternate_binding,
        &conflict_command,
        alternate_outcome,
        &alternate,
    );
    let conflict_record = record(alternate_binding, conflict_command, conflict_receipt);
    assert_eq!(
        live_store.persist_with_owner_control_receipt(
            &alternate,
            StoreMode::Enabled,
            &conflict_record
        ),
        Err(DurableControlStoreError::OwnerReceiptConflict)
    );

    let _ = fs::remove_dir_all(path.parent().expect("fixture parent"));
}
