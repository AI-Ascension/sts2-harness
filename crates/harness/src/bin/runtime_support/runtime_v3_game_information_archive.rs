// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use sts2_harness::context_memory::{
    EvidenceStatus, MAX_SOURCE_BYTES, MemoryCorpus, MemoryEntry, MemoryKind, MemoryRef,
    policy_owner::ActivePolicyBinding,
};
use sts2_harness::game_information::{LookupBinding, LookupSession};

use super::{RuntimeGameInformationOwner, valid_digest};

const HEADER_SCHEMA: &str = "ascension.runtime-v3.lookup-archive-pointer.v1";
const CHUNK_BYTES: usize = 48 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchivePointer {
    schema: String,
    manifest_sha256: String,
    manifest_bytes: usize,
    chunks: Vec<MemoryRef>,
}

pub(super) fn persist(
    owner: &RuntimeGameInformationOwner,
    expected_active: &ActivePolicyBinding,
    session: &LookupSession,
    corpus: &MemoryCorpus,
) -> Result<Option<(String, usize)>, String> {
    if session.records().is_empty() {
        return Ok(None);
    }
    let authority = owner
        .lock_lookup_snapshot(expected_active)
        .map_err(|_| String::from("selected memory policy changed before lookup archive"))?;
    let mut store = owner
        .archive_store
        .lock()
        .map_err(|_| String::from("lookup archive store is unavailable"))?;
    let archive = session
        .export_archive(corpus, &mut store)
        .map_err(|_| String::from("lookup archive failed validation or retention"))?;
    let chunks = archive.manifest.chunks(CHUNK_BYTES).collect::<Vec<_>>();
    if chunks.is_empty() || chunks.len() > 8 {
        return Err(String::from(
            "lookup archive exceeded its bounded chunk count",
        ));
    }
    let now = RuntimeGameInformationOwner::policy_now_timestamp();
    let expires =
        RuntimeGameInformationOwner::policy_timestamp_after(owner.archive_retention_seconds());
    let generation = corpus.generation().max(1);
    let admitted_seq = owner.policy_now_seconds();
    let mut references = Vec::with_capacity(chunks.len());
    for (index, bytes) in chunks.iter().enumerate() {
        let entry = archive_entry(
            owner,
            &format!("lookup-archive-chunk:{}:{index}", archive.sha256),
            "lookup-archive-chunk",
            "lookup-archive-chunk",
            bytes.to_vec(),
            generation,
            admitted_seq,
            &now,
            &expires,
            &session.binding().game_profile,
            false,
        )?;
        references.push(entry.reference());
        store
            .publish(entry)
            .map_err(|_| String::from("lookup archive chunk could not be persisted"))?;
    }
    let pointer = ArchivePointer {
        schema: HEADER_SCHEMA.to_owned(),
        manifest_sha256: archive.sha256.clone(),
        manifest_bytes: archive.manifest.len(),
        chunks: references,
    };
    let pointer_bytes =
        serde_json::to_vec(&pointer).map_err(|_| String::from("lookup archive index failed"))?;
    if pointer_bytes.len() > MAX_SOURCE_BYTES {
        return Err(String::from(
            "lookup archive index exceeded its storage bound",
        ));
    }
    let header = archive_entry(
        owner,
        &format!("lookup-archive:{}", archive.sha256),
        "lookup-archive",
        "lookup-archive-index",
        pointer_bytes,
        generation,
        admitted_seq,
        &now,
        &expires,
        &session.binding().game_profile,
        false,
    )?;
    store
        .publish(header)
        .map_err(|_| String::from("lookup archive index could not be persisted"))?;
    drop(store);
    drop(authority);
    Ok(Some((archive.sha256, chunks.len())))
}

pub(super) fn restore(
    owner: &RuntimeGameInformationOwner,
    expected_active: &ActivePolicyBinding,
    binding: LookupBinding,
    now: &str,
    expires_at: &str,
) -> Result<Option<(LookupSession, MemoryCorpus)>, String> {
    let authority = owner
        .lock_lookup_snapshot(expected_active)
        .map_err(|_| String::from("selected memory policy changed before lookup replay"))?;
    let selected = authority.snapshot();
    let store = owner
        .archive_store
        .lock()
        .map_err(|_| String::from("lookup archive store is unavailable"))?;
    let corpus = store
        .load_corpus()
        .map_err(|_| String::from("lookup archive store cannot be authenticated"))?;
    let mut headers = corpus
        .entries()
        .filter(|entry| entry.entry_id.starts_with("lookup-archive:"))
        .collect::<Vec<_>>();
    headers.sort_by_key(|entry| std::cmp::Reverse(entry.admitted_seq));
    let Some(header_entry) = headers.first() else {
        return Ok(None);
    };
    let header_bytes = corpus
        .read_content(
            &header_entry.reference(),
            "game-information",
            9_007_199_254_740_991,
            corpus.generation().max(1),
            now,
        )
        .map_err(|_| String::from("latest lookup archive is expired or unavailable"))?;
    let pointer: ArchivePointer = serde_json::from_slice(header_bytes)
        .map_err(|_| String::from("lookup archive index is malformed"))?;
    if pointer.schema != HEADER_SCHEMA
        || !valid_digest(&pointer.manifest_sha256)
        || header_entry.entry_id != format!("lookup-archive:{}", pointer.manifest_sha256)
        || pointer.manifest_bytes == 0
        || pointer.manifest_bytes > 262_144
        || pointer.chunks.is_empty()
        || pointer.chunks.len() > 8
    {
        return Err(String::from("lookup archive index identity is invalid"));
    }
    let mut manifest = Vec::with_capacity(pointer.manifest_bytes);
    for (index, reference) in pointer.chunks.iter().enumerate() {
        let expected_id = format!("lookup-archive-chunk:{}:{index}", pointer.manifest_sha256);
        if reference.entry_id != expected_id {
            return Err(String::from("lookup archive chunk identity is invalid"));
        }
        let chunk = corpus
            .read_content(
                reference,
                "game-information",
                9_007_199_254_740_991,
                corpus.generation().max(1),
                now,
            )
            .map_err(|_| String::from("lookup archive chunk is unavailable"))?;
        if chunk.len() > CHUNK_BYTES
            || manifest.len().saturating_add(chunk.len()) > pointer.manifest_bytes
        {
            return Err(String::from("lookup archive chunk exceeds its bound"));
        }
        manifest.extend_from_slice(chunk);
    }
    if manifest.len() != pointer.manifest_bytes
        || sts2_harness::sha256_hex(&manifest) != pointer.manifest_sha256
    {
        return Err(String::from(
            "lookup archive digest does not match its index",
        ));
    }
    let (session, corpus) = LookupSession::import_archive(
        &manifest,
        &pointer.manifest_sha256,
        binding,
        selected.policy.clone(),
        &store,
        now,
        expires_at,
    )
    .map_err(|_| String::from("lookup archive does not match current owner policy"))?;
    drop(store);
    drop(authority);
    Ok(Some((session, corpus)))
}

fn archive_entry(
    owner: &RuntimeGameInformationOwner,
    entry_id: &str,
    source_record_id: &str,
    content_ref: &str,
    content: Vec<u8>,
    generation: u64,
    admitted_seq: u64,
    created_at: &str,
    expires_at: &str,
    game_profile: &str,
    protected: bool,
) -> Result<MemoryEntry, String> {
    if content.is_empty() || content.len() > MAX_SOURCE_BYTES {
        return Err(String::from(
            "lookup archive artifact exceeded its entry bound",
        ));
    }
    Ok(MemoryEntry::new(
        owner.scope.clone(),
        entry_id,
        source_record_id,
        MemoryKind::StaticReference,
        EvidenceStatus::Reported,
        "game-information",
        content_ref,
        content,
        0,
        admitted_seq,
        generation,
        created_at,
        expires_at,
        game_profile,
        protected,
    ))
}
