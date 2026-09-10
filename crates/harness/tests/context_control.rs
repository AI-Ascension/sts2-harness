// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use sts2_harness::{
    ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextNote, ContextRenderError,
    ContextRenderer, ControlAuthority, ExoConfig, ExoError, ExoProvider, ExoSession, ExoTransport,
    ExoTransportError, GateStatus, ManagedRenderInput, ManagementProfile, ModelExecutionId,
};

const REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";

fn boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        state_id: "combat-1".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: REVISION.to_owned(),
        model_revision: "model-v1".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 0,
        control_version: 0,
    }
}

fn render_input() -> ManagedRenderInput {
    ManagedRenderInput {
        execution_id: "model-execution-7".to_owned(),
        state_id: "combat-1".to_owned(),
        generation: 1,
        observation: json!({
            "state_id":"combat-1",
            "generation":1,
            "visible_seed":"fixture-seed",
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
        legal_action_ids: vec!["combat.end-turn".to_owned()],
        objective: "survive".to_owned(),
        hard_constraints: vec!["visible state only".to_owned()],
    }
}

fn item() -> ContextItem {
    let bytes = b"historical fixture".to_vec();
    ContextItem {
        reference: ContextItemRef {
            item_id: "history-1".to_owned(),
            version: 1,
            sha256: sha256(&bytes),
        },
        kind: "history".to_owned(),
        bytes,
        protected: false,
        expires_at: 100,
    }
}

fn prepared() -> sts2_harness::PreparedContext {
    let item = item();
    let mut registry = BTreeMap::new();
    registry.insert("history-1:1".to_owned(), item.clone());
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items.push(item.reference.clone());
    draft.notes.push(ContextNote {
        reference: item.reference.clone(),
        attributed_to: "operator-1".to_owned(),
    });
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    ContextRenderer::enabled(&boundary(), render_input(), &draft, &registry, &config)
        .expect("prepared context")
}

#[test]
fn renderer_is_bounded_and_legacy_content_is_byte_stable() {
    let legacy = ContextRenderer::legacy(
        br#"{"observation":{"state_id":"combat-1"}}"#.to_vec(),
        br#"{"type":"object"}"#.to_vec(),
        br#"{"tools":[]}"#.to_vec(),
    )
    .expect("legacy");
    assert_eq!(legacy.profile(), ManagementProfile::Legacy);
    assert_eq!(
        legacy.provider_bytes(),
        br#"{"observation":{"state_id":"combat-1"}}"#
    );
    let enabled = prepared();
    assert_eq!(enabled.profile(), ManagementProfile::Enabled);
    assert!(
        enabled
            .provider_bytes()
            .windows(b"management_context".len())
            .any(|part| part == b"management_context")
    );
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    let exo = enabled.exo_request(ModelExecutionId::new(7).expect("execution"), &config);
    assert!(exo.is_ok());
    assert_ne!(enabled.manifest_sha256, legacy.manifest_sha256);
}

#[test]
fn compiled_exo_session_sends_the_approved_bytes_once() {
    #[derive(Debug, Clone)]
    struct FakeTransport {
        seen: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl ExoTransport for FakeTransport {
        fn exchange(
            &mut self,
            request: &[u8],
            _max_response_bytes: usize,
            _timeout_millis: u32,
        ) -> Result<Vec<u8>, ExoTransportError> {
            self.seen.lock().expect("lock").push(request.to_vec());
            Ok(br#"{"decision":"wait","rationale":"fixture"}"#.to_vec())
        }
        fn close(&mut self) -> Result<(), ExoTransportError> {
            Ok(())
        }
    }

    let prepared = prepared();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = ExoProvider::new(
        FakeTransport { seen: seen.clone() },
        ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config"),
    );
    let mut session = ExoSession::new(provider);
    session
        .decide_prepared(ModelExecutionId::new(7).expect("execution"), &prepared)
        .expect("decision");
    assert_eq!(
        seen.lock().expect("lock").as_slice(),
        &[prepared.provider_bytes().to_vec()]
    );
    let error = session
        .decide_prepared(ModelExecutionId::new(8).expect("execution"), &prepared)
        .expect_err("different reserved execution must fail");
    assert_eq!(error, ExoError::InvalidRequest);
    assert_eq!(seen.lock().expect("lock").len(), 1);
}

#[test]
fn renderer_rejects_unreadable_expired_and_unselected_pinned_content() {
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    let mut registry = BTreeMap::new();
    let invalid_bytes = vec![0xff, 0xfe];
    let invalid = ContextItem {
        reference: ContextItemRef {
            item_id: "invalid-utf8".to_owned(),
            version: 1,
            sha256: sha256(&invalid_bytes),
        },
        kind: "history".to_owned(),
        bytes: invalid_bytes,
        protected: false,
        expires_at: 100,
    };
    registry.insert("invalid-utf8:1".to_owned(), invalid.clone());
    let mut draft = ContextDraft::new("draft-invalid", "revision-1");
    draft.selected_items.push(invalid.reference.clone());
    assert_eq!(
        ContextRenderer::enabled_at(&boundary(), render_input(), &draft, &registry, &config, 1,)
            .expect_err("invalid UTF-8 must fail closed"),
        ContextRenderError::InvalidUtf8
    );

    let valid_bytes = b"expired fixture".to_vec();
    let expired = ContextItem {
        reference: ContextItemRef {
            item_id: "expired-1".to_owned(),
            version: 1,
            sha256: sha256(&valid_bytes),
        },
        kind: "history".to_owned(),
        bytes: valid_bytes,
        protected: false,
        expires_at: 2,
    };
    registry.insert("expired-1:1".to_owned(), expired.clone());
    draft.selected_items = vec![expired.reference.clone()];
    assert_eq!(
        ContextRenderer::enabled_at(&boundary(), render_input(), &draft, &registry, &config, 2,)
            .expect_err("expired content must fail closed"),
        ContextRenderError::ExpiredItem
    );

    draft.pinned_item_ids = vec!["expired-1".to_owned()];
    assert_eq!(
        ContextRenderer::enabled_at(&boundary(), render_input(), &draft, &registry, &config, 2,)
            .expect_err("an expired pinned item must not bypass its TTL"),
        ContextRenderError::ExpiredItem
    );

    draft.selected_items.clear();
    draft.pinned_item_ids = vec!["expired-1".to_owned()];
    assert_eq!(
        ContextRenderer::enabled_at(&boundary(), render_input(), &draft, &registry, &config, 1,)
            .expect_err("unselected pin must fail closed"),
        ContextRenderError::InvalidInput("pinned item is not selected")
    );
}

#[test]
fn renderer_rejects_content_digest_mismatch_for_selected_note_and_objective() {
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    let original = item();
    let tampered = ContextItem {
        bytes: b"tampered fixture".to_vec(),
        ..original.clone()
    };
    let mut registry = BTreeMap::new();
    registry.insert("history-1:1".to_owned(), tampered);

    let mut selected = ContextDraft::new("draft-selected", "revision-1");
    selected.selected_items.push(original.reference.clone());
    assert_eq!(
        ContextRenderer::enabled_at(
            &boundary(),
            render_input(),
            &selected,
            &registry,
            &config,
            1,
        )
        .expect_err("selected content digest mismatch must fail closed"),
        ContextRenderError::UnknownItem
    );

    let mut note = ContextDraft::new("draft-note", "revision-1");
    note.notes.push(ContextNote {
        reference: original.reference.clone(),
        attributed_to: "operator-1".to_owned(),
    });
    assert_eq!(
        ContextRenderer::enabled_at(&boundary(), render_input(), &note, &registry, &config, 1,)
            .expect_err("note content digest mismatch must fail closed"),
        ContextRenderError::UnknownItem
    );

    let mut objective = ContextDraft::new("draft-objective", "revision-1");
    objective.objective = Some(original.reference);
    assert_eq!(
        ContextRenderer::enabled_at(
            &boundary(),
            render_input(),
            &objective,
            &registry,
            &config,
            1,
        )
        .expect_err("objective content digest mismatch must fail closed"),
        ContextRenderError::UnknownItem
    );
}

#[test]
fn control_authority_recovers_pause_and_fences_boundary_changes() {
    let mut authority = ControlAuthority::new(boundary(), "revision-1");
    let paused = authority.request_pause("pause-1", 0).expect("pause");
    assert_eq!(authority.state().status, GateStatus::PausedReady);
    assert_eq!(
        authority
            .request_pause("pause-1", 0)
            .expect("idempotent pause"),
        paused
    );
    let journal = authority.export_journal().expect("journal");
    let mut recovered = ControlAuthority::recover(&journal).expect("recover");
    assert!(recovered.state().pause_latched);
    assert_eq!(recovered.state().boundary.controller_epoch, 2);
    assert_eq!(
        recovered
            .request_pause("pause-1", 0)
            .expect("recovered idempotent pause"),
        paused
    );
    let mut resumed_authority = recovered.clone();
    let expected = resumed_authority.state().boundary.clone();
    let resumed = resumed_authority
        .resume("resume-1", recovered.state().control_version, &expected)
        .expect("resume after recovery");
    assert_ne!(resumed.command_id, paused.command_id);
    recovered.advance_boundary();
    let error = recovered
        .resume(
            "resume-stale-1",
            recovered.state().control_version,
            &expected,
        )
        .expect_err("changed host boundary must block resume");
    assert_eq!(error, "preview_stale");
}

#[test]
fn ollama_renderer_preserves_legacy_and_marks_enabled_context() {
    let legacy = json!({"observation":{"state_id":"combat-1"}});
    assert_eq!(
        sts2_harness::ollama_user_content(&legacy).expect("legacy"),
        r#"{"state_id":"combat-1"}"#
    );
    let enabled = json!({
        "observation":{"state_id":"combat-1"},
        "management_profile":"management-enabled",
        "management_context":{"notes":[{"content":"operator note"}]}
    });
    let content = sts2_harness::ollama_user_content(&enabled).expect("enabled");
    assert!(content.contains("[management-context-v1]"));
    assert!(content.contains("operator note"));
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
