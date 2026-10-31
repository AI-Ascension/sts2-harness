// SPDX-License-Identifier: MIT
//! A distinct record namespace keeps complete manifests out of closed query-only feedback.
use super::*;

pub(in crate::game_information) fn handle_agent<P: LookupMcpPort>(
    session: &mut LookupSession,
    corpus: &mut MemoryCorpus,
    port: &mut P,
    binding: &LookupBinding,
    turn: LookupTurn,
) -> LookupFeedback {
    let result = match turn {
        LookupTurn::ContentManifest { operation_id } => session
            .content_manifest_port(binding, &operation_id, corpus, port)
            .map(|delivery| LookupFeedback::ContentManifest {
                record_ordinal: session.manifest_records.len() - 1,
                delivery: Box::new(delivery),
            }),
        LookupTurn::ReadManifest {
            record_ordinal,
            offset,
        } => read(session, corpus, record_ordinal, offset),
        _ => Err(LookupError::Invalid),
    };
    result.unwrap_or_else(LookupFeedback::Error)
}

pub(in crate::game_information) fn replay_agent(
    session: &mut LookupSession,
    corpus: &MemoryCorpus,
    turn: LookupTurn,
) -> Result<LookupFeedback, LookupError> {
    match turn {
        LookupTurn::ContentManifest { operation_id } => {
            let record_ordinal = session.manifest_replay_cursor;
            let record = session
                .manifest_records
                .get(record_ordinal)
                .ok_or(LookupError::Divergence)?;
            if record.operation_id != operation_id {
                return Err(LookupError::Divergence);
            }
            let outcome = session
                .replay_manifest_outcome(record, corpus)
                .map_err(|_| LookupError::Divergence)?;
            session.manifest_replay_cursor += 1;
            match outcome {
                ManifestReplayOutcome::Delivery(delivery) => Ok(LookupFeedback::ContentManifest {
                    record_ordinal,
                    delivery,
                }),
                ManifestReplayOutcome::RecordedFailure { error, .. } => {
                    Ok(LookupFeedback::Error(error))
                }
            }
        }
        LookupTurn::ReadManifest {
            record_ordinal,
            offset,
        } => {
            if record_ordinal >= session.manifest_replay_cursor {
                return Err(LookupError::Divergence);
            }
            let record = session
                .manifest_records
                .get(record_ordinal)
                .ok_or(LookupError::Divergence)?;
            let outcome = session
                .replay_manifest_outcome(record, corpus)
                .map_err(|_| LookupError::Divergence)?;
            if matches!(outcome, ManifestReplayOutcome::RecordedFailure { .. }) {
                return Ok(LookupFeedback::Error(LookupError::MissingRetention));
            }
            // Only a bounds refusal on an authenticated source is ordinary tool feedback.
            match read(session, corpus, record_ordinal, offset) {
                Ok(feedback) => Ok(feedback),
                Err(LookupError::Bounds) => Ok(LookupFeedback::Error(LookupError::Bounds)),
                Err(_) => Err(LookupError::Divergence),
            }
        }
        _ => Err(LookupError::Invalid),
    }
}

fn read(
    session: &LookupSession,
    corpus: &MemoryCorpus,
    record_ordinal: usize,
    offset: usize,
) -> Result<LookupFeedback, LookupError> {
    let record = session
        .manifest_records
        .get(record_ordinal)
        .ok_or(LookupError::MissingRetention)?;
    let bytes = session.read_retained_manifest(record, corpus, offset)?;
    Ok(LookupFeedback::ManifestBytes {
        record_ordinal,
        offset,
        total_bytes: record.source_bytes,
        bytes,
    })
}
