// SPDX-License-Identifier: MIT

// One-shot prepared-input consumption.  This stays beside the memory policy and records no
// provider or game effect; only the existing Phase 2 resume caller can use the returned bytes.

pub const MEMORY_RESUME_SCHEMA: &str = "ascension.context-memory.resume.v1";

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedResume {
    pub schema: String,
    pub approval_id: String,
    pub selection_id: String,
    pub phase2_revision_id: String,
    pub prepared_manifest_sha256: String,
    pub revocation_epoch: u64,
    pub expires_at: String,
    #[serde(skip)]
    rendered_bytes: Vec<u8>,
    #[serde(skip)]
    consumed: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ResumeSubmission {
    pub approval_id: String,
    pub phase2_revision_id: String,
    pub prepared_manifest_sha256: String,
    pub rendered_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumeOutcome {
    Submitted,
    AlreadySubmitted,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct FirstResumeLedger {
    prepared: BTreeMap<String, PreparedResume>,
}

// `rendered_bytes` is the prepared input itself: it is private and marked `#[serde(skip)]` so it
// never reaches serialized output, and serde attributes do not apply to `Debug`. Format the
// non-sensitive identity and shape metadata only, and end with `finish_non_exhaustive()` so a
// future sensitive field cannot silently re-enter this path.
impl std::fmt::Debug for PreparedResume {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedResume")
            .field("schema", &self.schema)
            .field("approval_id", &self.approval_id)
            .field("selection_id", &self.selection_id)
            .field("phase2_revision_id", &self.phase2_revision_id)
            .field("prepared_manifest_sha256", &self.prepared_manifest_sha256)
            .field("revocation_epoch", &self.revocation_epoch)
            .field("expires_at", &self.expires_at)
            .field("rendered_bytes_len", &self.rendered_bytes.len())
            .field("consumed", &self.consumed)
            .finish_non_exhaustive()
    }
}

// The submission carries the caller-supplied rendered bytes; report the length only.
impl std::fmt::Debug for ResumeSubmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResumeSubmission")
            .field("approval_id", &self.approval_id)
            .field("phase2_revision_id", &self.phase2_revision_id)
            .field("prepared_manifest_sha256", &self.prepared_manifest_sha256)
            .field("rendered_bytes_len", &self.rendered_bytes.len())
            .finish_non_exhaustive()
    }
}

// Formatting the ledger must not recurse into its prepared entries.
impl std::fmt::Debug for FirstResumeLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FirstResumeLedger")
            .field("prepared_count", &self.prepared.len())
            .finish_non_exhaustive()
    }
}

impl FirstResumeLedger {
    pub fn prepare(
        &mut self,
        approval: &MemoryApproval,
        selection: &SelectionManifest,
    ) -> Result<(), MemoryError> {
        if approval.state != ApprovalState::CommittedHeld
            || approval.selection_id != selection.selection_id
            || approval.selection_sha256 != selection.prepared_manifest_sha256
            || approval.phase2_prepared_manifest_sha256
                != selection.phase2_prepared_manifest_sha256
            || selection.phase2_revision_id != approval.planned_phase2_revision_id
            || selection.revocation_epoch != approval.revocation_epoch
            || !valid_timestamp(&selection.expires_at)
            || !valid_digest(&selection.prepared_manifest_sha256)
            || sha256_hex(&selection.rendered_bytes) != selection.prepared_manifest_sha256
        {
            return Err(MemoryError::StaleApproval);
        }
        let prepared = PreparedResume {
            schema: MEMORY_RESUME_SCHEMA.to_owned(),
            approval_id: approval.approval_id.clone(),
            selection_id: selection.selection_id.clone(),
            phase2_revision_id: selection.phase2_revision_id.clone(),
            prepared_manifest_sha256: selection.prepared_manifest_sha256.clone(),
            revocation_epoch: selection.revocation_epoch,
            expires_at: selection.expires_at.clone(),
            rendered_bytes: selection.rendered_bytes.clone(),
            consumed: false,
        };
        if let Some(existing) = self.prepared.get(&prepared.approval_id) {
            return if existing == &prepared {
                Ok(())
            } else {
                Err(MemoryError::Conflict)
            };
        }
        self.prepared.insert(prepared.approval_id.clone(), prepared);
        Ok(())
    }

    pub fn submit_first(
        &mut self,
        approval: &MemoryApproval,
        current_revocation_epoch: u64,
        now: &str,
        rendered_bytes: &[u8],
    ) -> Result<(ResumeOutcome, ResumeSubmission), MemoryError> {
        if !valid_timestamp(now) {
            return Err(MemoryError::InvalidQuery);
        }
        let prepared = self
            .prepared
            .get_mut(&approval.approval_id)
            .ok_or(MemoryError::StaleApproval)?;
        if approval.state != ApprovalState::CommittedHeld
            || approval.planned_phase2_revision_id != prepared.phase2_revision_id
            || approval.revocation_epoch != current_revocation_epoch
            || prepared.revocation_epoch != current_revocation_epoch
            || prepared.expires_at.as_str() <= now
            || sha256_hex(rendered_bytes) != prepared.prepared_manifest_sha256
            || rendered_bytes != prepared.rendered_bytes
        {
            return Err(MemoryError::StaleApproval);
        }
        if prepared.consumed {
            return Ok((
                ResumeOutcome::AlreadySubmitted,
                ResumeSubmission {
                    approval_id: prepared.approval_id.clone(),
                    phase2_revision_id: prepared.phase2_revision_id.clone(),
                    prepared_manifest_sha256: prepared.prepared_manifest_sha256.clone(),
                    rendered_bytes: Vec::new(),
                },
            ));
        }
        prepared.consumed = true;
        Ok((
            ResumeOutcome::Submitted,
            ResumeSubmission {
                approval_id: prepared.approval_id.clone(),
                phase2_revision_id: prepared.phase2_revision_id.clone(),
                prepared_manifest_sha256: prepared.prepared_manifest_sha256.clone(),
                rendered_bytes: prepared.rendered_bytes.clone(),
            },
        ))
    }

    pub fn is_consumed(&self, approval_id: &str) -> bool {
        self.prepared.get(approval_id).is_some_and(|item| item.consumed)
    }
}

#[cfg(test)]
mod resume_debug_tests {
    #![allow(clippy::expect_used)]

    use super::*;

    const BYTES: &[u8] = b"phase2-memory-v1\nmandatory\n--historical--\nsettled action";

    fn prepared() -> PreparedResume {
        PreparedResume {
            schema: MEMORY_RESUME_SCHEMA.to_owned(),
            approval_id: "approval-1".to_owned(),
            selection_id: "selection-1".to_owned(),
            phase2_revision_id: "revision-1".to_owned(),
            prepared_manifest_sha256: sha256_hex(BYTES),
            revocation_epoch: 3,
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            rendered_bytes: BYTES.to_vec(),
            consumed: false,
        }
    }

    // `rendered_bytes` is the prepared Phase 2 input: private on `PreparedResume` and marked
    // `#[serde(skip)]`. Serde's skip does not extend to `Debug`, so derived `Debug` would print
    // those bytes through `PreparedResume`, through the `FirstResumeLedger` map, and through
    // `ResumeSubmission`. Assert ordinary and alternate formatting withhold them, with positive
    // controls so the assertions cannot pass vacuously.
    #[test]
    fn debug_omits_prepared_resume_bytes() {
        let prepared = prepared();
        let mut ledger = FirstResumeLedger::default();
        ledger.prepared.insert("approval-1".to_owned(), prepared.clone());
        let submission = ResumeSubmission {
            approval_id: "approval-1".to_owned(),
            phase2_revision_id: "revision-1".to_owned(),
            prepared_manifest_sha256: sha256_hex(BYTES),
            rendered_bytes: BYTES.to_vec(),
        };

        let rendered_bytes = String::from_utf8_lossy(BYTES).into_owned();
        let sensitive = [
            rendered_bytes.clone(),
            "mandatory".to_owned(),
            "settled action".to_owned(),
            "rendered_bytes:".to_owned(),
        ];
        let formatted = [
            ("PreparedResume", "rendered_bytes_len", format!("{prepared:?}")),
            ("PreparedResume", "rendered_bytes_len", format!("{prepared:#?}")),
            ("FirstResumeLedger", "prepared_count", format!("{ledger:?}")),
            ("FirstResumeLedger", "prepared_count", format!("{ledger:#?}")),
            ("ResumeSubmission", "rendered_bytes_len", format!("{submission:?}")),
            ("ResumeSubmission", "rendered_bytes_len", format!("{submission:#?}")),
        ];
        for (type_name, control, text) in &formatted {
            assert!(text.contains(type_name), "{type_name} not formatted: {text}");
            assert!(text.contains(control), "no positive control for {type_name}: {text}");
            for needle in &sensitive {
                assert!(
                    !text.contains(needle.as_str()),
                    "formatting leaked prepared bytes {needle:?}: {text}"
                );
            }
        }

        // Serialization behaviour is unchanged: the skipped field still never serializes.
        let serialized = serde_json::to_string(&prepared).expect("serialize");
        assert!(!serialized.contains("rendered_bytes"), "{serialized}");
        assert!(!serialized.contains("settled action"), "{serialized}");
    }
}
