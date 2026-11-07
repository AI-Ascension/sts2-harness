// SPDX-License-Identifier: MIT
//! Mixed outcomes retain exact failure evidence and cannot renew imported retention.
use super::*;

#[test]
fn mixed_success_and_failure_archive_replays_each_manifest_outcome() -> TestResult {
    let (mut session, mut corpus, _) = capture(LookupError::Scope)?;
    let mut port = ManifestPort {
        raw: serde_json::to_vec(&manifest_value()?)?,
        calls: 0,
    };
    let binding = session.binding.clone();
    let delivery = session.content_manifest_port(&binding, "manifest-2", &mut corpus, &mut port)?;
    assert_eq!(port.calls, 1);
    let mut store = DurableMemoryStore::open(":memory:", binding.scope.clone(), [40; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let (mut restored, retained) = restore(&session, &archive, &store, NOW)?;
    let mut replay = failure_agent();
    replay.turns.insert(
        1,
        LookupTurn::ContentManifest {
            operation_id: "manifest-2".into(),
        },
    );
    assert_eq!(
        run_lookup_replay_tool_loop(&mut restored, &retained, &mut replay, &legal()?, 3)?,
        "play:card-17"
    );
    assert_eq!(
        replay.feedback,
        vec![
            LookupFeedback::Start,
            LookupFeedback::Error(LookupError::Scope),
            LookupFeedback::ContentManifest {
                record_ordinal: 1,
                delivery: Box::new(delivery),
            },
        ]
    );
    Ok(())
}

#[test]
fn reexport_copies_original_failure_window_and_cannot_recreate_missing_metadata() -> TestResult {
    let (session, corpus, _) = capture(LookupError::Divergence)?;
    let mut store = DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [41; 32])?;
    let archive = session.export_archive(&corpus, &mut store)?;
    let (restored, retained) = restore(&session, &archive, &store, NOW)?;
    let mut destination =
        DurableMemoryStore::open(":memory:", session.binding.scope.clone(), [42; 32])?;
    let copied = restored.export_archive(&retained, &mut destination)?;
    assert_eq!(copied.manifest, archive.manifest);
    assert_eq!(copied.sha256, archive.sha256);
    for entry in destination.load_corpus()?.entries() {
        assert_eq!(entry.created_at, NOW);
        assert_eq!(entry.expires_at, EXPIRY);
    }
    assert!(matches!(
        restore(&session, &copied, &destination, EXPIRY),
        Err(LookupError::MissingRetention)
    ));
    assert!(matches!(
        restored.export_archive(&corpus, &mut destination),
        Err(LookupError::MissingRetention)
    ));
    Ok(())
}
