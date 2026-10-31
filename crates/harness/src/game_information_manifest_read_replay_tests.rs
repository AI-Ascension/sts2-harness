// SPDX-License-Identifier: MIT
//! Read errors are replayable only after the corresponding retained record authenticates.
use super::*;

fn with_read(mut agent: Agent, offset: usize) -> Agent {
    agent.turns.insert(
        1,
        LookupTurn::ReadManifest {
            record_ordinal: 0,
            offset,
        },
    );
    agent
}

#[test]
fn failed_record_read_feedback_matches_live_without_creating_catalog_bytes() -> TestResult {
    for error in [
        LookupError::Scope,
        LookupError::Divergence,
        LookupError::MissingRetention,
    ] {
        let (mut session, mut corpus) = manifest_session(8192)?;
        let mut port = FailurePort {
            result: Err(error.clone()),
            calls: 0,
        };
        let mut live = with_read(failure_agent(), 0);
        assert_eq!(
            run_lookup_tool_loop(
                &mut session,
                &mut corpus,
                &mut port,
                &mut live,
                &legal()?,
                3
            )?,
            "play:card-17"
        );
        assert_eq!(port.calls, 1);
        assert_eq!(
            live.feedback,
            vec![
                LookupFeedback::Start,
                LookupFeedback::Error(error),
                LookupFeedback::Error(LookupError::MissingRetention)
            ]
        );
        let mut store =
            DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [43; 32])?;
        let archive = session.export_archive(&corpus, &mut store)?;
        let (mut restored, retained) = restore(&session, &archive, &store, NOW)?;
        let mut replay = with_read(failure_agent(), 0);
        assert_eq!(
            run_lookup_replay_tool_loop(&mut restored, &retained, &mut replay, &legal()?, 3)?,
            "play:card-17"
        );
        assert_eq!(replay.feedback, live.feedback);
        assert_eq!(
            restored.read_retained_manifest(&restored.manifest_records()[0], &retained, 0),
            Err(LookupError::MissingRetention)
        );
        for (live, replay) in live.feedback.iter().zip(&replay.feedback).skip(1) {
            assert_eq!(
                serde_json::to_vec(&crate::exo_lookup_wire::feedback_value(live, 8192)?)?,
                serde_json::to_vec(&crate::exo_lookup_wire::feedback_value(replay, 8192)?)?
            );
        }
    }
    Ok(())
}

#[test]
fn failed_record_read_rechecks_integrity_after_the_recorded_error_turn() -> TestResult {
    let (session, corpus, _) = capture(LookupError::Scope)?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [44; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    for mutation in [
        "revoked_policy",
        "revoked_metadata",
        "expired_metadata",
        "changed_record",
        "wrong_owner",
    ] {
        let (mut restored, mut retained) = restore(&session, &archive, &store, NOW)?;
        run_lookup_replay_tool_loop(&mut restored, &retained, &mut failure_agent(), &legal()?, 2)?;
        match mutation {
            "revoked_policy" => restored.policy.status = PolicyStatus::Revoked,
            "revoked_metadata" => {
                let references = retained
                    .entries()
                    .map(MemoryEntry::reference)
                    .collect::<Vec<_>>();
                retained.revoke(&references, NOW)?;
            }
            "expired_metadata" => restored.now = EXPIRY.into(),
            "changed_record" => restored.manifest_records[0].source_bytes = 1,
            "wrong_owner" => restored.binding.scope.run_id = "foreign".into(),
            _ => return Err("fixture".into()),
        }
        let mut replay = Agent {
            turns: VecDeque::from([
                LookupTurn::ReadManifest {
                    record_ordinal: 0,
                    offset: 0,
                },
                LookupTurn::Decide {
                    action_id: "play:card-17".into(),
                },
            ]),
            feedback: Vec::new(),
        };
        assert_eq!(
            run_lookup_replay_tool_loop(&mut restored, &retained, &mut replay, &legal()?, 2),
            Err(LookupError::Divergence),
            "{mutation}"
        );
        assert_eq!(replay.feedback, vec![LookupFeedback::Start], "{mutation}");
        assert_eq!(restored.manifest_replay_cursor, 1, "{mutation}");
    }
    Ok(())
}

#[test]
fn authenticated_successful_read_bounds_reproduce_live_tool_feedback() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    let raw = serde_json::to_vec(&manifest_value()?)?;
    let offset = raw.len() + 1;
    let mut port = ManifestPort { raw, calls: 0 };
    let mut live = with_read(failure_agent(), offset);
    assert_eq!(
        run_lookup_tool_loop(
            &mut session,
            &mut corpus,
            &mut port,
            &mut live,
            &legal()?,
            3
        )?,
        "play:card-17"
    );
    assert_eq!(port.calls, 1);
    assert_eq!(live.feedback[2], LookupFeedback::Error(LookupError::Bounds));
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [45; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let (mut restored, retained) = restore(&session, &archive, &store, NOW)?;
    let mut replay = with_read(failure_agent(), offset);
    assert_eq!(
        run_lookup_replay_tool_loop(&mut restored, &retained, &mut replay, &legal()?, 3)?,
        "play:card-17"
    );
    assert_eq!(replay.feedback, live.feedback);
    Ok(())
}

fn assert_manifest_read_diverges(
    session: &mut LookupSession,
    corpus: &MemoryCorpus,
    offset: usize,
    mutation: &str,
) -> TestResult {
    let mut replay = agent(offset);
    // The manifest turn was already authenticated before mutation.
    let _ = replay.turns.pop_front();
    assert_eq!(
        run_lookup_replay_tool_loop(session, corpus, &mut replay, &legal()?, 2),
        Err(LookupError::Divergence),
        "{mutation}, offset {offset}"
    );
    assert_eq!(replay.feedback, vec![LookupFeedback::Start], "{mutation}");
    assert_eq!(session.manifest_replay_cursor, 1, "{mutation}");
    assert_eq!(replay.turns.len(), 1, "the decision must not be consumed");
    Ok(())
}

#[test]
fn successful_record_read_rechecks_policy_and_sources_after_the_manifest_turn() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    let raw = serde_json::to_vec(&manifest_value()?)?;
    let delivery = read(&mut session, &mut corpus, raw.clone())?;
    assert!(delivery.record.error.is_none());
    assert!(!delivery.record.sources.is_empty());
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [46; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    for mutation in ["revoked_policy", "revoked_source", "expired_source"] {
        let (mut restored, mut retained) = restore(&session, &archive, &store, NOW)?;
        let mut manifest = failure_agent();
        assert_eq!(
            run_lookup_replay_tool_loop(&mut restored, &retained, &mut manifest, &legal()?, 2)?,
            "play:card-17"
        );
        assert_eq!(
            manifest.feedback,
            vec![
                LookupFeedback::Start,
                LookupFeedback::ContentManifest {
                    record_ordinal: 0,
                    delivery: Box::new(delivery.clone()),
                },
            ]
        );
        assert_eq!(restored.manifest_replay_cursor, 1);
        let record = &restored.manifest_records()[0];
        assert_eq!(restored.read_retained_manifest(record, &retained, 0)?, raw);
        match mutation {
            "revoked_policy" => restored.policy.status = PolicyStatus::Revoked,
            "revoked_source" => {
                retained.revoke(&record.sources, NOW)?;
            }
            "expired_source" => {
                for reference in &record.sources {
                    assert_eq!(
                        retained
                            .entry(reference)
                            .ok_or("fixture source")?
                            .expires_at,
                        EXPIRY
                    );
                }
                restored.now = EXPIRY.into();
            }
            _ => return Err("fixture".into()),
        }
        // Authentication failure must not become either bytes or ordinary bounds feedback.
        for offset in [0, raw.len() + 1] {
            assert_manifest_read_diverges(&mut restored, &retained, offset, mutation)?;
        }
    }
    Ok(())
}
