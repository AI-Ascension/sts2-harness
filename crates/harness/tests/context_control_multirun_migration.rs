// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use sts2_harness::{
    ContextBoundary, ContextControlStore, ContextDraft, ContextLifetimeLedger,
    ContextSourceDocument, ControlAuthority, DurableContextOwnerControlReceipt,
    DurableContextSourceSnapshot, DurableControlStoreError, StoreMode, context_source_digest,
};
use sts2_harness::management::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION,
    ContextBindingContinuity, ContextBindingGrants, ContextBindingState, ContextControlCommand,
    ContextControlCommandKind, ContextControlReceipt, ContextOwnerBinding,
};

#[derive(Debug, Eq, PartialEq)]
struct EncryptedRunRows {
    journal: (Vec<u8>, String),
    source: (Vec<u8>, String),
    lifetime: (Vec<u8>, String),
    receipts: Vec<(String, String, String, Vec<u8>, String)>,
}

fn authority(run_id: &str) -> ControlAuthority {
    ControlAuthority::new(
        ContextBoundary {
            run_id: run_id.to_owned(),
            episode_id: format!("episode-{run_id}"),
            agent_id: format!("agent-{run_id}"),
            state_id: format!("state-{run_id}"),
            generation: 1,
            observation_sha256: "a".repeat(64),
            catalog_sha256: "b".repeat(64),
            adapter_revision: "adapter-migration".to_owned(),
            model_revision: "model-migration".to_owned(),
            configuration_sha256: "c".repeat(64),
            output_schema_sha256: "d".repeat(64),
            controller_epoch: 1,
            gate_epoch: 0,
            control_version: 0,
        },
        "revision-1",
    )
}

fn path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ascension-context-control-{label}-{}.sqlite",
        uuid::Uuid::new_v4()
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = fs::remove_file(path.with_extension("sqlite-shm"));
}

fn mark_v1(path: &Path) {
    rusqlite::Connection::open(path)
        .expect("open schema marker")
        .execute_batch(
            "UPDATE context_control_meta SET value = 'ascension.context-control.sqlite.v1' WHERE key = 'schema';
             UPDATE context_control_meta SET value = '1' WHERE key = 'schema_version';
             DROP TABLE IF EXISTS context_control_owner_state;",
        )
        .expect("restore exact v1 marker and table set");
}

fn encrypted_rows(path: &Path, run_id: &str) -> EncryptedRunRows {
    let connection = rusqlite::Connection::open(path).expect("open encrypted run rows");
    let journal = connection
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
            [run_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("journal envelope");
    let source = connection
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_context_sources WHERE run_id = ?1",
            [run_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("source envelope");
    let lifetime = connection
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_lifetime WHERE run_id = ?1",
            [run_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("lifetime envelope");
    let mut statement = connection
        .prepare(
            "SELECT owner_id, command_digest, idempotency_digest, envelope, envelope_digest
             FROM context_control_owner_receipts WHERE run_id = ?1 ORDER BY command_digest",
        )
        .expect("prepare encrypted receipt rows");
    let receipts = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .expect("query encrypted receipt rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("read encrypted receipt rows");
    EncryptedRunRows {
        journal,
        source,
        lifetime,
        receipts,
    }
}

fn publish_source(store: &mut ContextControlStore, run_id: &str) -> String {
    let document = ContextSourceDocument {
        draft: ContextDraft::new(format!("draft-{run_id}"), "revision-1"),
        items: Default::default(),
    };
    let digest = context_source_digest(&document).expect("source digest");
    store
        .publish_context_source(&DurableContextSourceSnapshot {
            source_id: format!("source-{run_id}"),
            version: 1,
            digest: digest.clone(),
            document,
        })
        .expect("publish run source");
    digest
}

fn receipt_binding(run_id: &str, boundary: ContextBoundary) -> ContextOwnerBinding {
    ContextOwnerBinding {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.into(),
        owner_id: format!("owner-{run_id}"),
        owner_version: "1".into(),
        invocation_id: format!("{run_id}.invocation"),
        binding_id: format!("{run_id}.binding"),
        binding_version: 1,
        binding_digest: "e".repeat(64),
        context_ref: "context.1".into(),
        instance_id: "instance.1".into(),
        node_kind: "decide".into(),
        state: ContextBindingState::Available,
        workflow_run_id: run_id.into(),
        definition_digest: "f".repeat(64),
        graph_id: "graph.1".into(),
        node_id: "node.1".into(),
        node_execution_id: "execution.1".into(),
        boundary,
        lease_epoch: 7,
        snapshot_id: "snapshot.1".into(),
        approved_revision_id: "revision-1".into(),
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

fn persist_pause_receipt(
    store: &mut ContextControlStore,
    initial: &ControlAuthority,
    run_id: &str,
) -> (ContextControlCommand, DurableContextOwnerControlReceipt) {
    let binding = receipt_binding(run_id, initial.state().boundary.clone());
    let idempotency_key = format!("{run_id}.pause");
    let command = ContextControlCommand::Pause {
        idempotency_key: idempotency_key.clone(),
        expected_control_version: binding.boundary.control_version,
    };
    let mut paused = initial.clone();
    let outcome = paused
        .request_pause(&idempotency_key, binding.boundary.control_version)
        .expect("pause transition");
    let state = paused.state();
    let receipt = ContextControlReceipt {
        schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.into(),
        owner_id: binding.owner_id.clone(),
        invocation_id: binding.invocation_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_digest: binding.binding_digest.clone(),
        command: ContextControlCommandKind::Pause,
        command_id: outcome.command_id,
        idempotency_key: outcome.idempotency_key,
        effect: outcome.effect,
        control_version: outcome.control_version,
        plan_epoch: outcome.plan_epoch,
        controller_epoch: state.boundary.controller_epoch,
        gate_epoch: state.boundary.gate_epoch,
        boundary: state.boundary.clone(),
        revision_id: None,
        preview_manifest_digest: None,
        approved_manifest_digest: None,
    };
    let record = DurableContextOwnerControlReceipt {
        owner_id: binding.owner_id.clone(),
        actor_subject: format!("operator-{run_id}"),
        binding,
        command: command.clone(),
        receipt,
    };
    store
        .persist_with_owner_control_receipt(&paused, StoreMode::Enabled, &record)
        .expect("persist run-scoped encrypted control receipt");
    (command, record)
}

fn assert_v1_marker_and_no_owner_table(path: &Path) {
    let connection = rusqlite::Connection::open(path).expect("inspect refused migration");
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM context_control_meta WHERE key = 'schema'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema marker"),
        "ascension.context-control.sqlite.v1"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM context_control_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema version"),
        "1"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'context_control_owner_state'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("owner table count"),
        0
    );
}

#[test]
fn migration_authenticates_one_run_and_preserves_other_runs_with_independent_keys() {
    let path = path("multi-run");
    let key_a = [41_u8; 32];
    let key_b = [42_u8; 32];
    let key_c = [43_u8; 32];
    let digest_a;
    let digest_b;
    let (command_b, expected_receipt_b);
    {
        let mut store_a = ContextControlStore::create(
            &path,
            key_a,
            "run-a",
            &authority("run-a"),
            StoreMode::Enabled,
        )
        .expect("create run A");
        digest_a = publish_source(&mut store_a, "run-a");
        store_a
            .persist_lifetime(&ContextLifetimeLedger::new())
            .expect("persist run A lifetime state");
    }
    {
        let authority_b = authority("run-b");
        let mut store_b = ContextControlStore::create(
            &path,
            key_b,
            "run-b",
            &authority_b,
            StoreMode::Enabled,
        )
        .expect("create run B using its own key");
        digest_b = publish_source(&mut store_b, "run-b");
        (command_b, expected_receipt_b) =
            persist_pause_receipt(&mut store_b, &authority_b, "run-b");
        store_b
            .persist_lifetime(&ContextLifetimeLedger::new())
            .expect("persist run B lifetime state");
    }
    mark_v1(&path);
    let rows_a_before = encrypted_rows(&path, "run-a");
    let rows_b_before = encrypted_rows(&path, "run-b");
    assert_eq!(rows_b_before.receipts.len(), 1, "B has a real encrypted owner receipt before migration");

    assert!(matches!(
        ContextControlStore::open(&path, key_c, "run-c"),
        Err(DurableControlStoreError::MigrationRequired)
    ));
    assert_v1_marker_and_no_owner_table(&path);
    assert_eq!(encrypted_rows(&path, "run-a"), rows_a_before);
    assert_eq!(encrypted_rows(&path, "run-b"), rows_b_before);
    assert!(matches!(
        ContextControlStore::open(&path, [44_u8; 32], "run-a"),
        Err(DurableControlStoreError::AuthenticationFailed)
    ));
    assert_v1_marker_and_no_owner_table(&path);
    assert_eq!(encrypted_rows(&path, "run-a"), rows_a_before);
    assert_eq!(encrypted_rows(&path, "run-b"), rows_b_before);

    {
        let store_a = ContextControlStore::open(&path, key_a, "run-a").expect("migrate run A");
        assert_eq!(
            store_a
                .load_context_source("source-run-a", 1, &digest_a)
                .expect("load migrated A source")
                .expect("A source exists")
                .document
                .draft
                .id,
            "draft-run-a"
        );
    }
    assert_eq!(encrypted_rows(&path, "run-b"), rows_b_before);
    {
        let store_b = ContextControlStore::open(&path, key_b, "run-b").expect("open B key");
        assert_eq!(store_b.load().expect("authenticate B journal").state().boundary.run_id, "run-b");
        assert_eq!(
            store_b
                .load_context_source("source-run-b", 1, &digest_b)
                .expect("authenticate B source")
                .expect("B source exists")
                .document
                .draft
                .id,
            "draft-run-b"
        );
        assert!(store_b.load_lifetime().expect("authenticate B lifetime").is_some());
        assert_eq!(
            store_b
                .lookup_owner_control_receipt(
                    &expected_receipt_b.owner_id,
                    &expected_receipt_b.actor_subject,
                    &command_b,
                )
                .expect("authenticate B receipt"),
            Some(expected_receipt_b)
        );
    }
    assert_eq!(encrypted_rows(&path, "run-b"), rows_b_before);
    ContextControlStore::create(
        &path,
        key_c,
        "run-c",
        &authority("run-c"),
        StoreMode::Enabled,
    )
    .expect("create new run after global migration");
    cleanup(&path);
}

include!("context_control_multirun_failures.rs");
