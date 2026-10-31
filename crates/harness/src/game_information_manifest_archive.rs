// SPDX-License-Identifier: MIT
//! Additive archive v2; the closed query-only archive v1 remains unchanged.
use super::*;
use crate::context_memory::DurableMemoryStore;
use std::collections::BTreeSet;

const SCHEMA: &str = "ascension.game-information-archive.v2-content-manifest";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestArchive {
    schema: String,
    protocol_profile: String,
    schema_digest: String,
    binding: LookupBinding,
    /// An unchanged v1 archive when query transcripts also exist.
    query_archive: Option<Value>,
    manifest_records: Vec<manifest::ContentManifestRecord>,
}

impl LookupSession {
    /// Query-only sessions keep their exact v1 archive; manifest sessions select explicit v2.
    pub fn export_archive(
        &self,
        corpus: &MemoryCorpus,
        store: &mut DurableMemoryStore,
    ) -> Result<LookupArchive, LookupError> {
        if self.manifest_records.is_empty() {
            return self.export_query_archive(corpus, store);
        }
        self.policy
            .validate(corpus)
            .map_err(|_| LookupError::Scope)?;
        validate_records(&self.manifest_records, &self.binding)?;
        for record in &self.manifest_records {
            if record.error.is_none() {
                self.replay_content_manifest(record, corpus)?;
            }
        }
        let query_archive = if self.records.is_empty() {
            if !self.bootstrap_records.is_empty() {
                // A bootstrap transcript still needs the v1 query evidence it was selected from.
                return Err(LookupError::MissingRetention);
            }
            publish_dependencies(self, corpus, store)?;
            None
        } else {
            Some(validation::decode_strict(
                &self.export_query_archive(corpus, store)?.manifest,
            )?)
        };
        let mut binding = self.binding.clone();
        if query_archive.is_none() {
            // A catalog cannot attest a live snapshot. Only the unchanged query archive can.
            binding.snapshot = None;
        }
        let manifest = ManifestArchive {
            schema: SCHEMA.to_owned(),
            protocol_profile: manifest::PROFILE.to_owned(),
            schema_digest: manifest::SCHEMA_DIGEST.to_owned(),
            binding,
            query_archive,
            manifest_records: self.manifest_records.clone(),
        };
        let bytes = serde_json::to_vec(&manifest).map_err(|_| LookupError::Invalid)?;
        if bytes.len() > validation::MAX_MESSAGE_BYTES {
            return Err(LookupError::Bounds);
        }
        for record in &self.manifest_records {
            if record.error.is_some() {
                manifest::publish_failure(self, record, corpus, store)?;
            }
            for reference in &record.sources {
                store
                    .publish(
                        corpus
                            .entry(reference)
                            .cloned()
                            .ok_or(LookupError::MissingRetention)?,
                    )
                    .map_err(|_| LookupError::Retention)?;
            }
        }
        let persisted = store
            .load_corpus()
            .map_err(|_| LookupError::MissingRetention)?;
        self.policy
            .validate(&persisted)
            .map_err(|_| LookupError::Retention)?;
        for record in &self.manifest_records {
            self.replay_manifest_outcome(record, &persisted)?;
        }
        Ok(LookupArchive {
            sha256: crate::sha256_hex(&bytes),
            manifest: bytes,
        })
    }

    /// The caller supplies the expected digest and owner; archive data grants no live authority.
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
        if value["schema"] != SCHEMA {
            return Self::import_query_archive(
                bytes,
                expected_sha256,
                binding,
                policy,
                store,
                now,
                expires_at,
            );
        }
        let archive: ManifestArchive =
            serde_json::from_value(value).map_err(|_| LookupError::Invalid)?;
        if archive.protocol_profile != manifest::PROFILE
            || archive.schema_digest != manifest::SCHEMA_DIGEST
            || !archive.binding.same_owner(&binding)
        {
            return Err(LookupError::Divergence);
        }
        validate_records(&archive.manifest_records, &binding)?;
        let (mut session, corpus) = match archive.query_archive {
            Some(query) => {
                let raw = serde_json::to_vec(&query).map_err(|_| LookupError::Invalid)?;
                let digest = crate::sha256_hex(&raw);
                Self::import_query_archive(&raw, &digest, binding, policy, store, now, expires_at)?
            }
            None => {
                if archive.binding.snapshot.is_some() {
                    return Err(LookupError::Divergence);
                }
                let corpus = store
                    .load_corpus()
                    .map_err(|_| LookupError::MissingRetention)?;
                let mut binding = binding;
                binding.snapshot = None;
                (
                    Self::new(binding, policy, &corpus, now, expires_at)?,
                    corpus,
                )
            }
        };
        if session.binding.snapshot != archive.binding.snapshot {
            return Err(LookupError::Divergence);
        }
        for record in &archive.manifest_records {
            if let manifest::ManifestReplayOutcome::RecordedFailure { retention, .. } =
                session.replay_manifest_outcome(record, &corpus)?
            {
                session
                    .manifest_imported_failures
                    .insert(record.operation_id.clone(), retention);
            }
        }
        session.manifest_records = archive.manifest_records;
        Ok((session, corpus))
    }
}

fn validate_records(
    records: &[manifest::ContentManifestRecord],
    binding: &LookupBinding,
) -> Result<(), LookupError> {
    if records.is_empty() || records.len() > manifest::MAX_RECORDS {
        return Err(LookupError::Bounds);
    }
    let mut operations = BTreeSet::new();
    for record in records {
        manifest::validation::record(record, binding)?;
        if !operations.insert(&record.operation_id) {
            return Err(LookupError::Divergence);
        }
    }
    Ok(())
}

fn publish_dependencies(
    session: &LookupSession,
    corpus: &MemoryCorpus,
    store: &mut DurableMemoryStore,
) -> Result<(), LookupError> {
    let mut entries = BTreeMap::new();
    for reference in &session.policy.approved_summary_catalog {
        archive::collect_dependency(reference, corpus, &mut entries)?;
    }
    let mut entries = entries.into_values().collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        (
            entry.admitted_seq,
            entry.observed_seq,
            entry.lineage_depth,
            entry.entry_id.clone(),
        )
    });
    for entry in entries {
        store.publish(entry).map_err(|_| LookupError::Retention)?;
    }
    Ok(())
}
