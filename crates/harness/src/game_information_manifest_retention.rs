// SPDX-License-Identifier: MIT
//! Atomic corpus admission and exact ordered source reconstruction.
use super::*;
use crate::context_memory::{
    DurableMemoryStore, EvidenceStatus, MAX_SOURCE_BYTES, MemoryEntry, MemoryKind,
};

pub(super) fn stage(
    session: &LookupSession,
    record: &ContentManifestRecord,
    bytes: &[u8],
    corpus: &MemoryCorpus,
) -> Result<(MemoryCorpus, Vec<MemoryRef>), LookupError> {
    let mut staged = corpus.clone();
    let identity = crate::sha256_hex(serde_json::to_vec(record).map_err(|_| LookupError::Invalid)?);
    let mut sources = Vec::new();
    for (index, chunk) in bytes.chunks(MAX_SOURCE_BYTES).enumerate() {
        let id = format!("manifest:{identity}:{index}");
        let entry = MemoryEntry::new(
            record.binding.scope.clone(),
            &id,
            &record.operation_id,
            MemoryKind::StaticReference,
            EvidenceStatus::Reported,
            "game-information",
            &id,
            chunk.to_vec(),
            index as u64,
            index as u64,
            staged.generation() + 1,
            &session.now,
            &session.expires_at,
            &record.binding.game_profile,
            false,
        );
        sources.push(entry.reference());
        staged.admit(entry).map_err(|_| LookupError::Retention)?;
    }
    Ok((staged, sources))
}

pub(super) fn retained(
    record: &ContentManifestRecord,
    corpus: &MemoryCorpus,
    now: &str,
) -> Result<Vec<u8>, LookupError> {
    let mut bytes = Vec::with_capacity(record.source_bytes);
    for (index, reference) in record.sources.iter().enumerate() {
        let entry = corpus
            .entry(reference)
            .ok_or(LookupError::MissingRetention)?;
        if entry.scope != record.binding.scope || entry.game_profile != record.binding.game_profile
        {
            return Err(LookupError::Scope);
        }
        if entry.created_at.as_str() > now {
            return Err(LookupError::MissingRetention);
        }
        let chunk = corpus
            .read_content(
                reference,
                "game-information",
                9_007_199_254_740_991,
                corpus.generation(),
                now,
            )
            .map_err(|_| LookupError::MissingRetention)?;
        let expected = (record.source_bytes - index * MAX_SOURCE_BYTES).min(MAX_SOURCE_BYTES);
        if chunk.len() != expected || crate::sha256_hex(chunk) != reference.sha256 {
            return Err(LookupError::Divergence);
        }
        bytes.extend_from_slice(chunk);
    }
    if bytes.len() != record.source_bytes || Some(crate::sha256_hex(&bytes)) != record.source_sha256
    {
        return Err(LookupError::Divergence);
    }
    Ok(bytes)
}

fn failure_identity(record: &ContentManifestRecord) -> Result<(MemoryRef, Vec<u8>), LookupError> {
    let bytes = serde_json::to_vec(record).map_err(|_| LookupError::Invalid)?;
    let digest = crate::sha256_hex(&bytes);
    Ok((
        MemoryRef::new(format!("manifest-failure:{digest}"), 1, digest),
        bytes,
    ))
}

/// Failure metadata is authenticated historical data, never a successful producer source.
pub(in crate::game_information) fn publish_failure(
    session: &LookupSession,
    record: &ContentManifestRecord,
    corpus: &MemoryCorpus,
    store: &mut DurableMemoryStore,
) -> Result<(), LookupError> {
    let entry = if let Some(original) = session.manifest_imported_failures.get(&record.operation_id)
    {
        let reference = retained_failure(record, corpus, &session.now)?;
        if original != &reference {
            return Err(LookupError::Divergence);
        }
        corpus
            .entry(&reference)
            .cloned()
            .ok_or(LookupError::MissingRetention)?
    } else {
        let (reference, bytes) = failure_identity(record)?;
        MemoryEntry::new(
            record.binding.scope.clone(),
            &reference.entry_id,
            "manifest-failure",
            MemoryKind::OperatorNote,
            EvidenceStatus::Reported,
            "game-information",
            &reference.entry_id,
            bytes,
            0,
            0,
            corpus.generation(),
            &session.now,
            &session.expires_at,
            &record.binding.game_profile,
            false,
        )
    };
    store
        .publish(entry)
        .map(|_| ())
        .map_err(|_| LookupError::Retention)
}

pub(super) fn retained_failure(
    record: &ContentManifestRecord,
    corpus: &MemoryCorpus,
    now: &str,
) -> Result<MemoryRef, LookupError> {
    let (reference, bytes) = failure_identity(record)?;
    let entry = corpus
        .entry(&reference)
        .ok_or(LookupError::MissingRetention)?;
    if entry.scope != record.binding.scope || entry.game_profile != record.binding.game_profile {
        return Err(LookupError::Scope);
    }
    if entry.kind != MemoryKind::OperatorNote
        || entry.evidence != EvidenceStatus::Reported
        || entry.source_record_id != "manifest-failure"
        || entry.created_at.as_str() > now
    {
        return Err(LookupError::MissingRetention);
    }
    let retained = corpus
        .read_content(
            &reference,
            "game-information",
            9_007_199_254_740_991,
            corpus.generation(),
            now,
        )
        .map_err(|_| LookupError::MissingRetention)?;
    if retained != bytes {
        return Err(LookupError::Divergence);
    }
    Ok(reference)
}
