// SPDX-License-Identifier: MIT
//! Recorded failures are outcomes; fresh policy/source corruption still aborts replay.
use super::*;

#[path = "game_information_manifest_failure_archive_tests.rs"]
mod failure_archive_tests;

#[path = "game_information_manifest_read_replay_tests.rs"]
mod read_replay_tests;

struct FailurePort {
    result: Result<(String, Vec<u8>), LookupError>,
    calls: usize,
}
impl LookupMcpPort for FailurePort {
    fn information_correlation(&self) -> Result<String, LookupError> {
        Ok("42".into())
    }
    fn call_information(&mut self, _: &str, _: &Value) -> Result<Vec<u8>, LookupError> {
        Err(LookupError::Transport)
    }
    fn information_content_manifest(&mut self) -> Result<(String, Vec<u8>), LookupError> {
        self.calls += 1;
        self.result.clone()
    }
}

fn failure_agent() -> Agent {
    Agent {
        turns: VecDeque::from([
            LookupTurn::ContentManifest {
                operation_id: "manifest-1".into(),
            },
            LookupTurn::Decide {
                action_id: "play:card-17".into(),
            },
        ]),
        feedback: Vec::new(),
    }
}

fn capture(
    error: LookupError,
) -> Result<(LookupSession, MemoryCorpus, Vec<LookupFeedback>), Box<dyn std::error::Error>> {
    let (mut session, mut corpus) = manifest_session(8192)?;
    let result = if error == LookupError::Scope {
        let mut value = manifest_value()?;
        value["manifest"]["inventory_revision"] = json!("d".repeat(64));
        Ok(("42".into(), serde_json::to_vec(&value)?))
    } else {
        Err(error.clone())
    };
    let mut port = FailurePort { result, calls: 0 };
    let mut agent = failure_agent();
    assert_eq!(
        run_lookup_tool_loop(
            &mut session,
            &mut corpus,
            &mut port,
            &mut agent,
            &legal()?,
            2
        )?,
        "play:card-17"
    );
    assert_eq!(
        port.calls, 1,
        "failure must follow actual live port admission"
    );
    assert_eq!(session.manifest_records()[0].error, Some(error.clone()));
    assert!(session.manifest_records()[0].sources.is_empty());
    assert_eq!(agent.feedback[1], LookupFeedback::Error(error));
    Ok((session, corpus, agent.feedback))
}

fn restore(
    session: &LookupSession,
    archive: &LookupArchive,
    store: &DurableMemoryStore,
    now: &str,
) -> Result<(LookupSession, MemoryCorpus), LookupError> {
    LookupSession::import_archive(
        &archive.manifest,
        &archive.sha256,
        session.binding.clone(),
        session.policy.clone(),
        store,
        now,
        "2026-09-18T00:00:00Z",
    )
}

fn failure_round_trip(error: LookupError) -> TestResult {
    let (session, corpus, live_feedback) = capture(error)?;
    let path = std::env::temp_dir().join(format!(
        "sts2-manifest-failure-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let path_text = path.to_str().ok_or("fixture path")?;
    let archive = {
        let mut store =
            DurableMemoryStore::open(path_text, session.binding.scope.clone(), [35; 32])?;
        session.export_archive(&corpus, &mut store)?
    };
    let store = DurableMemoryStore::open(path_text, session.binding.scope.clone(), [35; 32])?;
    let (mut restored, retained) = restore(&session, &archive, &store, NOW)?;
    let mut replay = failure_agent();
    assert_eq!(
        run_lookup_replay_tool_loop(&mut restored, &retained, &mut replay, &legal()?, 2)?,
        "play:card-17"
    );
    assert_eq!(replay.feedback, live_feedback);
    let bytes = |feedback: &LookupFeedback| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(serde_json::to_vec(
            &crate::exo_lookup_wire::feedback_value(feedback, 8192)?,
        )?)
    };
    assert_eq!(bytes(&replay.feedback[1])?, bytes(&live_feedback[1])?);
    drop(store);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn live_scope_failure_survives_encrypted_restart_and_agent_replay() -> TestResult {
    failure_round_trip(LookupError::Scope)
}

#[test]
fn live_divergence_failure_survives_encrypted_restart_and_agent_replay() -> TestResult {
    failure_round_trip(LookupError::Divergence)
}

#[test]
fn live_missing_retention_failure_survives_encrypted_restart_and_agent_replay() -> TestResult {
    failure_round_trip(LookupError::MissingRetention)
}

#[test]
fn failure_replay_rechecks_record_owner_policy_metadata_and_retention() -> TestResult {
    let (session, corpus, _) = capture(LookupError::Scope)?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [36; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    for mutation in [
        "record_scope",
        "corpus_scope",
        "source_claim",
        "changed_outcome",
        "revoked_policy",
        "revoked_metadata",
        "expired_metadata",
    ] {
        let (mut restored, mut retained) = restore(&session, &archive, &store, NOW)?;
        match mutation {
            "record_scope" => restored.manifest_records[0].binding.scope.run_id = "foreign".into(),
            "corpus_scope" => {
                retained = MemoryCorpus::new(MemoryScope::new(
                    "project-1",
                    "foreign",
                    "episode-1",
                    "agent-1",
                ))?
            }
            "source_claim" => restored.manifest_records[0].source_bytes = 1,
            "changed_outcome" => restored.manifest_records[0].error = Some(LookupError::Divergence),
            "revoked_policy" => restored.policy.status = PolicyStatus::Revoked,
            "revoked_metadata" => {
                let references = retained
                    .entries()
                    .map(MemoryEntry::reference)
                    .collect::<Vec<_>>();
                retained.revoke(&references, NOW)?;
            }
            "expired_metadata" => restored.now = EXPIRY.into(),
            _ => return Err("fixture".into()),
        }
        let mut replay = failure_agent();
        assert_eq!(
            run_lookup_replay_tool_loop(&mut restored, &retained, &mut replay, &legal()?, 2),
            Err(LookupError::Divergence),
            "{mutation}"
        );
        assert_eq!(replay.feedback, vec![LookupFeedback::Start], "{mutation}");
        assert_eq!(restored.manifest_replay_cursor, 0, "{mutation}");
    }
    Ok(())
}

#[test]
fn failure_archive_requires_exact_retained_metadata_and_original_window() -> TestResult {
    let (session, corpus, _) = capture(LookupError::Scope)?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [37; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    // A valid corpus generation alone cannot stand in for the archived failure's metadata.
    let mut unrelated =
        DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [37; 32])?;
    for entry in corpus.entries() {
        unrelated.publish(entry.clone())?;
    }
    assert!(matches!(
        restore(&session, &archive, &unrelated, NOW),
        Err(LookupError::MissingRetention)
    ));
    assert!(matches!(
        restore(&session, &archive, &store, EXPIRY),
        Err(LookupError::MissingRetention)
    ));
    assert!(matches!(
        restore(&session, &archive, &store, "2026-09-14T00:00:00Z"),
        Err(LookupError::MissingRetention)
    ));
    let (mut imported, retained) = restore(&session, &archive, &store, NOW)?;
    imported.now = EXPIRY.into();
    let mut destination =
        DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [39; 32])?;
    assert!(matches!(
        imported.export_archive(&retained, &mut destination),
        Err(LookupError::MissingRetention)
    ));
    let mut changed: Value = serde_json::from_slice(&archive.manifest)?;
    changed["manifest_records"][0]["error"] = json!(LookupError::Divergence);
    let bytes = serde_json::to_vec(&changed)?;
    let forged = LookupArchive {
        sha256: crate::sha256_hex(&bytes),
        manifest: bytes,
    };
    assert!(matches!(
        restore(&session, &forged, &store, NOW),
        Err(LookupError::MissingRetention)
    ));
    Ok(())
}

#[test]
fn an_unadmitted_request_or_forged_source_failure_cannot_be_replayed() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    session.invalidate();
    let binding = session.binding.clone();
    let mut port = FailurePort {
        result: Err(LookupError::Scope),
        calls: 0,
    };
    assert_eq!(
        session.content_manifest_port(&binding, "manifest-1", &mut corpus, &mut port),
        Err(LookupError::MissingCapability)
    );
    assert_eq!(port.calls, 0);
    assert!(session.manifest_records().is_empty());
    let mut replay = failure_agent();
    assert_eq!(
        run_lookup_replay_tool_loop(&mut session, &corpus, &mut replay, &legal()?, 2),
        Err(LookupError::Divergence)
    );

    let (mut session, mut corpus) = manifest_session(8192)?;
    read(
        &mut session,
        &mut corpus,
        serde_json::to_vec(&manifest_value()?)?,
    )?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [38; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    for retain_source_claims in [false, true] {
        let (mut restored, retained) = restore(&session, &archive, &store, NOW)?;
        let record = &mut restored.manifest_records[0];
        record.error = Some(LookupError::Scope);
        if !retain_source_claims {
            record.correlation_id = None;
            record.source_sha256 = None;
            record.sources.clear();
            record.source_bytes = 0;
            record.view_sha256 = None;
        }
        let mut replay = failure_agent();
        assert_eq!(
            run_lookup_replay_tool_loop(&mut restored, &retained, &mut replay, &legal()?, 2),
            Err(LookupError::Divergence)
        );
        assert_eq!(replay.feedback, vec![LookupFeedback::Start]);
        assert_eq!(restored.manifest_replay_cursor, 0);
    }
    Ok(())
}
