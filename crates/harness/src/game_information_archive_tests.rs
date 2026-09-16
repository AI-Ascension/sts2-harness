// SPDX-License-Identifier: MIT
//! Original synthetic archive fixtures; no producer implementation or game assets.
use super::*;
use crate::context_memory::*;
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const NOW: &str = "2026-09-15T00:00:00Z";
const EXPIRY: &str = "2026-09-16T00:00:00Z";

fn fixture() -> Result<(LookupSession, MemoryCorpus, Vec<u8>), Box<dyn std::error::Error>> {
    let scope = MemoryScope::new(
        "archive-project",
        "archive-run",
        "archive-episode",
        "archive-agent",
    );
    let mut corpus = MemoryCorpus::new(scope.clone())?;
    corpus.admit(MemoryEntry::new(
        scope.clone(),
        "archive-bootstrap",
        "bootstrap",
        MemoryKind::OperatorNote,
        EvidenceStatus::Reported,
        "game-information",
        "bootstrap",
        b"original synthetic fixture".to_vec(),
        0,
        0,
        1,
        NOW,
        EXPIRY,
        "archive-profile",
        false,
    ))?;
    let policy = MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "archive-policy".to_owned(),
        version: 1,
        scope: scope.clone(),
        mode: PolicyMode::BoundedPerDecision,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("archive-approval".to_owned()),
        corpus_generation: 1,
        rolling_same_episode_sources: true,
        cross_scope: false,
        approved_summary_catalog: vec![],
        ranker_version: "none".to_owned(),
        query_derivation_version: "v1".to_owned(),
        max_candidates: 4,
        max_results: 4,
        max_selected: 4,
        optional_byte_budget: 4096,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "v1".to_owned(),
    };
    let binding = LookupBinding {
        scope,
        game_profile: "archive-profile".to_owned(),
        content_manifest_id: "archive-content".to_owned(),
        locale: "en".to_owned(),
        authority_epoch: 1,
        snapshot: None,
    };
    let mut session = LookupSession::new(binding, policy, &corpus, NOW, EXPIRY)?;
    let request = json!({
        "protocol_version":PROFILE,"schema_digest":SCHEMA_DIGEST,"correlation_id":"archive-correlation",
        "provenance":{"artifact":"sts2-protocol/game-information-query-v1","source":"schemas/game-information-query-v1.schema.json","generator":"hand-authored"},
        "kind":"query_request","result":null,"capabilities":null,"error":null,
        "query":{"query_kind":"list","entity_kind":"card","target":{"definition_ref":null,"instance_ref":null},
            "filters":{"display_name":null,"namespaced_ids":[],"definition_refs":[],"instance_ids":[]},
            "projection":"summary","detail_level":"summary","fields":["display_name"],
            "binding":{"mode":"static","content_manifest_id":"archive-content","locale":"en",
                "visibility_scope":"public","instance_ref":null,"snapshot_ref":null},"parent_observation":null,
            "limits":{"page_items":4,"item_bytes":4096,"page_bytes":65536,"text_bytes":4096},"cursor":null}
    });
    let mut response = request.clone();
    response["kind"] = json!("query_response");
    response["result"] = json!({"read_only":true,"parent_observation":null,"result_generation":null,
        "page":{"items":[],"final_page":true,"next_cursor":null,"cursor_binding":null,"coverage":"complete",
            "total_count_known":true,"total_count":0,"limits":request["query"]["limits"],
            "ordering":{"key":"definition_ref","direction":"ascending","algorithm":"identity_bytes","deterministic":true}}});
    let size = serde_json::to_vec(&response["result"]["page"])?.len();
    response["result"]["page"]["accounting"] =
        json!({"item_count":0,"item_bytes":0,"payload_bytes":2,"page_bytes":size,"text_bytes":0});
    let mut raw = serde_json::to_vec_pretty(&response)?;
    raw.extend_from_slice(b"\n \n");
    session.capabilities = Some(json!({"max_message_bytes":65536,"fields":["display_name"]}));
    session.accept("archive-operation", 0, request, raw.clone(), &mut corpus)?;
    Ok((session, corpus, raw))
}

#[test]
fn encrypted_restart_preserves_exact_source_and_replays_without_transport() -> TestResult {
    let (session, corpus, raw) = fixture()?;
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("lookup-archive-{}.sqlite", uuid::Uuid::new_v4()));
    let path_text = path.to_str().ok_or("synthetic path")?;
    let archive = {
        let mut store =
            DurableMemoryStore::open(path_text, session.binding.scope.clone(), [23; 32])?;
        session.export_archive(&corpus, &mut store)?
    };
    let record = session.records[0].clone();
    let binding = session.binding.clone();
    let policy = session.policy.clone();
    drop(session);
    drop(corpus);
    {
        let store = DurableMemoryStore::open(path_text, binding.scope.clone(), [23; 32])?;
        let (mut restored, corpus) = LookupSession::import_archive(
            &archive.manifest,
            &archive.sha256,
            binding,
            policy,
            &store,
            NOW,
            EXPIRY,
        )?;
        assert_eq!(restored.records(), std::slice::from_ref(&record));
        assert_eq!(records::retained(&record, &corpus, NOW)?, raw);
        let delivery = restored.replay(&record, &record.request, &corpus)?;
        assert_eq!(delivery.record.view_sha256, record.view_sha256);
        assert!(restored.capabilities.is_none());
        struct ArchivedAgent {
            request: Value,
            received: bool,
        }
        impl LookupAgentPort for ArchivedAgent {
            fn next_turn(
                &mut self,
                input: LookupAgentInput<'_>,
            ) -> Result<LookupTurn, LookupError> {
                if let LookupFeedback::Data { delivery, .. } = input.feedback {
                    self.received = delivery.record.view_sha256.is_some();
                }
                Ok(if self.received {
                    LookupTurn::Decide {
                        action_id: "play:card-17".to_owned(),
                    }
                } else {
                    LookupTurn::Query {
                        operation_id: String::from("archive-operation"),
                        request: serde_json::to_vec(&self.request)
                            .map_err(|_| LookupError::Invalid)?,
                    }
                })
            }
        }
        let legal = crate::EpisodeLegalActionSet::new(
            "state-42",
            42,
            vec![crate::EpisodeLegalAction::new(
                "play:card-17",
                crate::ActionKind::PlayCard,
            )?],
        )?;
        let mut agent = ArchivedAgent {
            request: record.request.clone(),
            received: false,
        };
        assert_eq!(
            run_lookup_replay_tool_loop(&mut restored, &corpus, &mut agent, &legal, 2)?,
            "play:card-17"
        );
        assert!(agent.received, "replay must deliver archived data to agent");
    }
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn missing_sources_tampered_manifest_and_wrong_owner_fail_closed() -> TestResult {
    let (session, corpus, _) = fixture()?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [24; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let missing = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [24; 32])?;
    let restore =
        |bytes: &[u8], digest: &str, binding: LookupBinding, store: &DurableMemoryStore| {
            LookupSession::import_archive(
                bytes,
                digest,
                binding,
                session.policy.clone(),
                store,
                NOW,
                EXPIRY,
            )
        };
    assert!(
        restore(
            &archive.manifest,
            &archive.sha256,
            session.binding.clone(),
            &missing
        )
        .is_err()
    );
    let mut tampered = archive.manifest.clone();
    tampered.push(b' ');
    assert!(matches!(
        restore(&tampered, &archive.sha256, session.binding.clone(), &store),
        Err(LookupError::Divergence)
    ));
    let mut wrong = session.binding.clone();
    wrong.game_profile = "other-profile".to_owned();
    assert!(matches!(
        restore(&archive.manifest, &archive.sha256, wrong, &store),
        Err(LookupError::Divergence)
    ));
    let mut wrong = session.binding.clone();
    wrong.scope = MemoryScope::new(
        "another-project",
        "archive-run",
        "archive-episode",
        "archive-agent",
    );
    assert!(matches!(
        restore(&archive.manifest, &archive.sha256, wrong, &store),
        Err(LookupError::Divergence)
    ));
    let mut value: Value = serde_json::from_slice(&archive.manifest)?;
    value["records"][0]["source_sha256"] = json!("0".repeat(64));
    let bytes = serde_json::to_vec(&value)?;
    assert!(matches!(
        restore(
            &bytes,
            &crate::sha256_hex(&bytes),
            session.binding.clone(),
            &store
        ),
        Err(LookupError::Divergence)
    ));
    Ok(())
}

#[test]
fn static_history_survives_owner_snapshot_observation() -> TestResult {
    let (mut session, corpus, _) = fixture()?;
    let old = session.records[0].clone();
    let instance = json!({"instance_id":"archive-instance","run_id":"archive-run","epoch":1,
        "entity_kind":"card","entity_id":"archive-card"});
    session.observe_snapshot(
        json!({"instance_ref":instance,"snapshot_id":"archive-snapshot","state_generation":2}),
    );
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [26; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let (restored, corpus) = LookupSession::import_archive(
        &archive.manifest,
        &archive.sha256,
        session.binding.clone(),
        session.policy.clone(),
        &store,
        NOW,
        EXPIRY,
    )?;
    assert!(restored.binding.snapshot.is_some());
    assert!(old.binding.snapshot.is_none());
    restored.replay(&old, &old.request, &corpus)?;
    Ok(())
}

#[test]
fn failed_attempt_and_successful_retry_preserve_distinct_record_ordinals() -> TestResult {
    let (mut session, corpus, _) = fixture()?;
    let mut failed = session.records[0].clone();
    failed.source = None;
    failed.source_sha256 = None;
    failed.source_bytes = 0;
    failed.view_sha256 = None;
    failed.error = Some(LookupError::Transport);
    session.records.insert(0, failed);
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [27; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let (restored, corpus) = LookupSession::import_archive(
        &archive.manifest,
        &archive.sha256,
        session.binding.clone(),
        session.policy.clone(),
        &store,
        NOW,
        EXPIRY,
    )?;
    assert_eq!(restored.records(), session.records());
    assert_eq!(restored.records[0].error, Some(LookupError::Transport));
    restored.replay(&restored.records[1], &restored.records[1].request, &corpus)?;
    Ok(())
}

#[test]
fn error_only_archive_requires_persisted_policy_dependencies() -> TestResult {
    let (mut session, corpus, _) = fixture()?;
    let record = &mut session.records[0];
    record.source = None;
    record.source_sha256 = None;
    record.source_bytes = 0;
    record.view_sha256 = None;
    record.error = Some(LookupError::Transport);
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [28; 32])?;
    assert!(matches!(
        session.export_archive(&corpus, &mut store),
        Err(LookupError::Retention)
    ));
    let bootstrap = corpus
        .entries()
        .find(|entry| entry.entry_id == "archive-bootstrap")
        .ok_or("missing synthetic bootstrap")?;
    store.publish(bootstrap.clone())?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let (restored, corpus) = LookupSession::import_archive(
        &archive.manifest,
        &archive.sha256,
        session.binding.clone(),
        session.policy.clone(),
        &store,
        NOW,
        EXPIRY,
    )?;
    assert_eq!(restored.records(), session.records());
    assert!(matches!(
        restored.replay(&restored.records[0], &restored.records[0].request, &corpus),
        Err(LookupError::Transport)
    ));
    Ok(())
}

#[test]
fn duplicate_manifest_keys_and_size_limit_fail_closed() -> TestResult {
    let (session, corpus, _) = fixture()?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [25; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let text = String::from_utf8(archive.manifest)?;
    let duplicate = text
        .replacen("{", "{\"schema\":\"duplicate\",", 1)
        .into_bytes();
    assert!(
        LookupSession::import_archive(
            &duplicate,
            &crate::sha256_hex(&duplicate),
            session.binding.clone(),
            session.policy.clone(),
            &store,
            NOW,
            EXPIRY
        )
        .is_err()
    );
    let bytes = vec![b' '; validation::MAX_MESSAGE_BYTES + 1];
    assert!(matches!(
        LookupSession::import_archive(
            &bytes,
            &crate::sha256_hex(&bytes),
            session.binding.clone(),
            session.policy.clone(),
            &store,
            NOW,
            EXPIRY
        ),
        Err(LookupError::Bounds)
    ));
    Ok(())
}
