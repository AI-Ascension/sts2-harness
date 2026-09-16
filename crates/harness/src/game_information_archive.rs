// SPDX-License-Identifier: MIT
//! Bounded replay manifest paired with the existing encrypted memory store.
//! The caller owns artifact publication and supplies its independently pinned manifest digest.
use super::*;
use crate::context_memory::{DurableMemoryStore, MemoryEntry, MemoryRef};
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "game_information_archive_tests.rs"]
mod tests;

const ARCHIVE_SCHEMA: &str = "ascension.game-information-archive.v1";
const MAX_ARCHIVE_RECORDS: usize = 256;

/// Publish both this manifest and the companion encrypted memory store.
/// The digest detects divergence; it does not grant authority to an unknown artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupArchive {
    pub manifest: Vec<u8>,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    protocol_profile: String,
    schema_digest: String,
    binding: LookupBinding,
    records: Vec<LookupRecord>,
}

impl LookupSession {
    /// Validates complete replay sources before publishing them through the existing encrypted
    /// store. A failed publication never returns a usable manifest.
    pub fn export_archive(
        &self,
        corpus: &MemoryCorpus,
        store: &mut DurableMemoryStore,
    ) -> Result<LookupArchive, LookupError> {
        let manifest = Manifest {
            schema: ARCHIVE_SCHEMA.to_owned(),
            protocol_profile: PROFILE.to_owned(),
            schema_digest: SCHEMA_DIGEST.to_owned(),
            binding: self.binding.clone(),
            records: self.records.clone(),
        };
        validate_manifest(&manifest, &self.binding)?;
        let bytes = serde_json::to_vec(&manifest).map_err(|_| LookupError::Invalid)?;
        if bytes.len() > validation::MAX_MESSAGE_BYTES {
            return Err(LookupError::Bounds);
        }
        // Validate every record before performing any publication.
        for record in &manifest.records {
            if record.error.is_none() {
                self.replay(record, &record.request, corpus)?;
            }
        }
        let mut dependencies = BTreeMap::new();
        for reference in &self.policy.approved_summary_catalog {
            collect_dependency(reference, corpus, &mut dependencies)?;
        }
        let mut dependencies = dependencies.into_values().collect::<Vec<_>>();
        dependencies.sort_by_key(|entry| {
            (
                entry.admitted_seq,
                entry.observed_seq,
                entry.lineage_depth,
                entry.entry_id.clone(),
            )
        });
        for entry in dependencies {
            store.publish(entry).map_err(|_| LookupError::Retention)?;
        }
        for record in &manifest.records {
            if let Some(reference) = &record.source {
                let entry = corpus
                    .entry(reference)
                    .ok_or(LookupError::MissingRetention)?;
                store
                    .publish(entry.clone())
                    .map_err(|_| LookupError::Retention)?;
            }
        }
        let persisted = store
            .load_corpus()
            .map_err(|_| LookupError::MissingRetention)?;
        self.policy
            .validate(&persisted)
            .map_err(|_| LookupError::Retention)?;
        for record in &manifest.records {
            if record.error.is_none() {
                self.replay(record, &record.request, &persisted)?;
            }
        }
        Ok(LookupArchive {
            sha256: crate::sha256_hex(&bytes),
            manifest: bytes,
        })
    }

    /// Restore against an owner-selected binding, approved policy and independently retained
    /// manifest digest. Capabilities are deliberately unnegotiated; replay has no transport.
    #[allow(clippy::too_many_arguments)]
    pub fn import_archive(
        bytes: &[u8],
        expected_sha256: &str,
        binding: LookupBinding,
        policy: MemoryPolicy,
        store: &DurableMemoryStore,
        now: &str,
        expires_at: &str,
    ) -> Result<(Self, MemoryCorpus), LookupError> {
        if bytes.len() > validation::MAX_MESSAGE_BYTES {
            return Err(LookupError::Bounds);
        }
        if crate::sha256_hex(bytes) != expected_sha256 {
            return Err(LookupError::Divergence);
        }
        let value = validation::decode_strict(bytes)?;
        let manifest: Manifest = serde_json::from_value(value).map_err(|_| LookupError::Invalid)?;
        validate_manifest(&manifest, &binding)?;
        let corpus = store
            .load_corpus()
            .map_err(|_| LookupError::MissingRetention)?;
        let mut session = Self::new(binding, policy, &corpus, now, expires_at)?;
        for record in &manifest.records {
            if record.error.is_none() {
                session.replay(record, &record.request, &corpus)?;
            }
        }
        session.records = manifest.records;
        Ok((session, corpus))
    }
}

fn collect_dependency(
    reference: &MemoryRef,
    corpus: &MemoryCorpus,
    entries: &mut BTreeMap<MemoryRef, MemoryEntry>,
) -> Result<(), LookupError> {
    if entries.contains_key(reference) {
        return Ok(());
    }
    let entry = corpus
        .entry(reference)
        .cloned()
        .ok_or(LookupError::MissingRetention)?;
    entry
        .validate_contract()
        .map_err(|_| LookupError::MissingRetention)?;
    if entry.scope != *corpus.scope() {
        return Err(LookupError::Scope);
    }
    for parent in &entry.parents {
        collect_dependency(
            &MemoryRef::new(&parent.entry_id, parent.version, &parent.sha256),
            corpus,
            entries,
        )?;
    }
    entries.insert(reference.clone(), entry);
    Ok(())
}

fn validate_manifest(manifest: &Manifest, binding: &LookupBinding) -> Result<(), LookupError> {
    if manifest.schema != ARCHIVE_SCHEMA
        || manifest.protocol_profile != PROFILE
        || manifest.schema_digest != SCHEMA_DIGEST
        || manifest.binding != *binding
    {
        return Err(LookupError::Divergence);
    }
    if manifest.records.is_empty() || manifest.records.len() > MAX_ARCHIVE_RECORDS {
        return Err(LookupError::Bounds);
    }
    let mut identities = BTreeSet::new();
    for record in &manifest.records {
        validation::validate_request(&record.request)?;
        if !record.binding.same_owner(binding)
            || record.schema != "ascension.game-information-record.v1"
            || record.operation_id.is_empty()
            || record.operation_id.len() > 128
            || !record
                .operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
            || record.page_sequence >= MAX_PAGES
            || record.request["query"]["binding"]["content_manifest_id"]
                != binding.content_manifest_id
            || record.request["query"]["binding"]["locale"] != binding.locale
            || (record.request["query"]["binding"]["mode"] == "live"
                && record.binding.snapshot.as_ref()
                    != Some(&record.request["query"]["binding"]["snapshot_ref"]))
            || (record.error.is_none()
                && !identities.insert((record.operation_id.clone(), record.page_sequence)))
        {
            return Err(LookupError::Divergence);
        }
        if record.error.is_some() {
            if record.source.is_some()
                || record.source_sha256.is_some()
                || record.source_bytes != 0
                || record.view_sha256.is_some()
            {
                return Err(LookupError::Divergence);
            }
        } else if record.source.is_none()
            || record.source_sha256.is_none()
            || record.view_sha256.is_none()
            || record.source_bytes == 0
            || record.source_bytes > crate::context_memory::MAX_SOURCE_BYTES
        {
            return Err(LookupError::MissingRetention);
        }
    }
    Ok(())
}
