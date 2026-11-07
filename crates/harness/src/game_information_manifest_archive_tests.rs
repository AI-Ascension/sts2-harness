// SPDX-License-Identifier: MIT
//! Encrypted restart, transcript replay and owner-bound failure tests.
use super::*;
use std::collections::VecDeque;

#[path = "game_information_manifest_failure_replay_tests.rs"]
mod failure_replay_tests;

struct Agent {
    turns: VecDeque<LookupTurn>,
    feedback: Vec<LookupFeedback>,
}
impl LookupAgentPort for Agent {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
        self.feedback.push(input.feedback.clone());
        self.turns.pop_front().ok_or(LookupError::Bounds)
    }
}
fn agent(offset: usize) -> Agent {
    Agent {
        turns: VecDeque::from([
            LookupTurn::ContentManifest {
                operation_id: "manifest-1".to_owned(),
            },
            LookupTurn::ReadManifest {
                record_ordinal: 0,
                offset,
            },
            LookupTurn::Decide {
                action_id: "play:card-17".to_owned(),
            },
        ]),
        feedback: Vec::new(),
    }
}
fn legal() -> Result<crate::EpisodeLegalActionSet, Box<dyn std::error::Error>> {
    Ok(crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?)
}

#[test]
fn encrypted_manifest_restart_replays_exact_agent_feedback_without_mcp() -> TestResult {
    let (mut session, mut corpus) = manifest_session(7000)?;
    let raw = large_manifest()?;
    let offset = raw.len() - 17;
    let mut port = ManifestPort {
        raw: raw.clone(),
        calls: 0,
    };
    let mut live_agent = agent(offset);
    assert_eq!(
        run_lookup_tool_loop(
            &mut session,
            &mut corpus,
            &mut port,
            &mut live_agent,
            &legal()?,
            3
        )?,
        "play:card-17"
    );
    assert_eq!(port.calls, 1);
    assert!(matches!(
        &live_agent.feedback[1],
        LookupFeedback::ContentManifest {
            record_ordinal: 0,
            ..
        }
    ));
    assert!(
        matches!(&live_agent.feedback[2], LookupFeedback::ManifestBytes { bytes, .. } if bytes == &raw[offset..])
    );
    let path = std::env::temp_dir().join(format!(
        "sts2-manifest-archive-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let path_text = path.to_str().ok_or("synthetic path")?;
    let binding = session.binding.clone();
    let policy = session.policy.clone();
    let archive = {
        let mut store = DurableMemoryStore::open(path_text, binding.scope.clone(), [31; 32])?;
        session.export_archive(&corpus, &mut store)?
    };
    let encoded: Value = serde_json::from_slice(&archive.manifest)?;
    assert_eq!(
        encoded["schema"],
        "ascension.game-information-archive.v2-content-manifest"
    );
    assert!(encoded["query_archive"].is_null());
    drop(session);
    drop(corpus);
    {
        let store = DurableMemoryStore::open(path_text, binding.scope.clone(), [31; 32])?;
        let (mut restored, corpus) = LookupSession::import_archive(
            &archive.manifest,
            &archive.sha256,
            binding,
            policy,
            &store,
            NOW,
            EXPIRY,
        )?;
        assert!(restored.capabilities.is_none());
        assert!(restored.binding.snapshot.is_none());
        let mut replay_agent = agent(offset);
        assert_eq!(
            run_lookup_replay_tool_loop(&mut restored, &corpus, &mut replay_agent, &legal()?, 3)?,
            "play:card-17"
        );
        assert_eq!(replay_agent.feedback, live_agent.feedback);
    }
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn query_and_manifest_archive_keeps_the_entire_closed_v1_transcript() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    // The original query profile accepts BCP-47 locale syntax; never widen that closed schema.
    session.binding.locale = "en".to_owned();
    let mut query = request(false, "17");
    query["query"]["binding"]["content_manifest_id"] = json!(session.binding.content_manifest_id);
    query["query"]["binding"]["locale"] = json!(session.binding.locale);
    let mut result = response(&query, false)?;
    result["result"]["page"]["items"] = json!([]);
    result["result"]["page"]["total_count"] = json!(0);
    account(&mut result)?;
    let binding = session.binding.clone();
    session.query(
        &binding,
        "static-1",
        &serde_json::to_vec(&query)?,
        &mut corpus,
        |_, _| serde_json::to_vec(&result).map_err(|_| LookupError::Invalid),
    )?;
    let mut store = DurableMemoryStore::open(":memory:", binding.scope.clone(), [32; 32])?;
    let old_archive = session.export_archive(&corpus, &mut store)?;
    let old: Value = serde_json::from_slice(&old_archive.manifest)?;
    assert_eq!(old["schema"], "ascension.game-information-archive.v1");
    assert!(old.get("manifest_records").is_none());
    let mut manifest = manifest_value()?;
    manifest["manifest"]["locale"] = json!("en");
    read(&mut session, &mut corpus, serde_json::to_vec(&manifest)?)?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let value: Value = serde_json::from_slice(&archive.manifest)?;
    assert_eq!(value["query_archive"], old);
    let (restored, corpus) = LookupSession::import_archive(
        &archive.manifest,
        &archive.sha256,
        binding,
        session.policy.clone(),
        &store,
        NOW,
        EXPIRY,
    )?;
    assert_eq!(restored.records().len(), 1);
    assert_eq!(restored.manifest_records().len(), 1);
    restored.replay(&restored.records()[0], &query, &corpus)?;
    restored.replay_content_manifest(&restored.manifest_records()[0], &corpus)?;
    Ok(())
}

#[test]
fn archive_rejects_forged_scope_digest_chunks_profile_and_snapshot() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    read(&mut session, &mut corpus, large_manifest()?)?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [33; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let value: Value = serde_json::from_slice(&archive.manifest)?;
    let restore = |bytes: &[u8], digest: &str, store: &DurableMemoryStore| {
        LookupSession::import_archive(
            bytes,
            digest,
            session.binding.clone(),
            session.policy.clone(),
            store,
            NOW,
            EXPIRY,
        )
    };
    assert!(restore(&archive.manifest, &"0".repeat(64), &store).is_err());
    let empty = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [33; 32])?;
    assert!(restore(&archive.manifest, &archive.sha256, &empty).is_err());
    for mutation in [
        "scope",
        "profile",
        "digest",
        "missing_chunk",
        "duplicate_chunk",
        "snapshot",
        "record_snapshot",
        "unknown_member",
    ] {
        let mut changed = value.clone();
        match mutation {
            "scope" => changed["binding"]["scope"]["run_id"] = json!("foreign"),
            "profile" => changed["protocol_profile"] = json!("game-information-query-v1"),
            "digest" => changed["manifest_records"][0]["source_sha256"] = json!("0".repeat(64)),
            "missing_chunk" => {
                changed["manifest_records"][0]["sources"] =
                    json!([changed["manifest_records"][0]["sources"][0]])
            }
            "duplicate_chunk" => {
                changed["manifest_records"][0]["sources"][1] =
                    changed["manifest_records"][0]["sources"][0].clone()
            }
            "snapshot" => changed["binding"]["snapshot"] = json!({"state_generation":42}),
            "record_snapshot" => {
                changed["manifest_records"][0]["binding"]["snapshot"] =
                    json!({"state_generation":42})
            }
            "unknown_member" => changed["untrusted_extension"] = json!(true),
            _ => return Err("fixture".into()),
        }
        let bytes = serde_json::to_vec(&changed)?;
        assert!(
            restore(&bytes, &crate::sha256_hex(&bytes), &store).is_err(),
            "{mutation}"
        );
    }
    Ok(())
}

#[test]
fn replay_rejects_changed_operation_and_reads_before_the_recorded_turn() -> TestResult {
    for turn in [
        LookupTurn::ContentManifest {
            operation_id: "other".into(),
        },
        LookupTurn::ReadManifest {
            record_ordinal: 0,
            offset: 0,
        },
    ] {
        let (mut session, mut corpus) = manifest_session(8192)?;
        read(
            &mut session,
            &mut corpus,
            serde_json::to_vec(&manifest_value()?)?,
        )?;
        let mut agent = Agent {
            turns: VecDeque::from([turn]),
            feedback: Vec::new(),
        };
        assert_eq!(
            run_lookup_replay_tool_loop(&mut session, &corpus, &mut agent, &legal()?, 1),
            Err(LookupError::Divergence)
        );
    }
    Ok(())
}

#[test]
fn missing_capability_failure_can_be_archived_without_inventing_a_catalog() -> TestResult {
    struct LegacyPort;
    impl LookupMcpPort for LegacyPort {
        fn information_correlation(&self) -> Result<String, LookupError> {
            Ok("42".into())
        }
        fn call_information(&mut self, _: &str, _: &Value) -> Result<Vec<u8>, LookupError> {
            Err(LookupError::Transport)
        }
    }
    let (mut session, mut corpus) = manifest_session(8192)?;
    let binding = session.binding.clone();
    assert_eq!(
        session.content_manifest_port(&binding, "manifest-1", &mut corpus, &mut LegacyPort),
        Err(LookupError::MissingCapability)
    );
    let mut store = DurableMemoryStore::open(":memory:", binding.scope.clone(), [34; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let (restored, corpus) = LookupSession::import_archive(
        &archive.manifest,
        &archive.sha256,
        binding,
        session.policy.clone(),
        &store,
        NOW,
        EXPIRY,
    )?;
    assert!(restored.manifest_records()[0].sources.is_empty());
    assert_eq!(
        restored.replay_content_manifest(&restored.manifest_records()[0], &corpus),
        Err(LookupError::MissingCapability)
    );
    Ok(())
}
