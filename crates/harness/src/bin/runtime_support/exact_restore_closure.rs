// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{
    BranchContinuationStrategyPlan, ExactArtifactStore, ExactAssurance, ExactCheckpointId,
    ExactCheckpointReference, ExactStateDigest, VerificationOutcome,
};

use super::{
    CLOSURE_DOMAIN, MAX_ARTIFACT_REFERENCES, MAX_BLOB_BYTES, MAX_CLOSURE_BYTES, ProfilePins,
};
use crate::runtime_support::branch_continuation_runtime::SelectedBranchContinuation;

#[path = "exact_restore_closure_validation.rs"]
mod validation;
use validation::{
    protocol_references, read_verified_blob, required_manifest_string,
    validate_checkpoint_identity, validate_restore_references,
};

#[derive(Clone, Debug)]
pub(crate) struct TransferBlob {
    pub(super) digest: String,
    pub(super) bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedClosure {
    pub(crate) manifest_digest: String,
    pub(crate) manifest_size_bytes: u64,
    pub(crate) closure_digest: String,
    pub(crate) exact_state_digest: String,
    pub(crate) checkpoint_id: String,
    pub(crate) boundary: Value,
    pub(crate) artifact_references: Vec<Value>,
    pub(crate) transfer_blobs: Vec<TransferBlob>,
    pub(crate) artifact_reference_count: usize,
    pub(crate) distinct_blob_count: usize,
    pub(crate) aggregate_closure_bytes: u64,
    pub(crate) compatibility_digest: String,
    pub(crate) coverage_contract_digest: String,
}

impl VerifiedClosure {
    pub(crate) fn prepare(
        selected: &SelectedBranchContinuation,
        artifact_store_path: &std::path::Path,
        pins: ProfilePins,
    ) -> Result<Self, String> {
        let BranchContinuationStrategyPlan::ExactRestore {
            checkpoint,
            restore_closure,
        } = selected.strategy()
        else {
            return Err(String::from(
                "exact restore closure was requested for a prefix replay branch",
            ));
        };
        let checkpoint_id = ExactCheckpointId::parse(&checkpoint.artifact_id)
            .map_err(|_| String::from("selected checkpoint id is invalid"))?;
        let store = ExactArtifactStore::new(artifact_store_path);
        let manifest = store
            .read_manifest(&checkpoint_id)
            .map_err(|error| format!("selected checkpoint manifest is unavailable: {error}"))?;
        if manifest.is_empty() || manifest.len() > MAX_BLOB_BYTES {
            return Err(String::from(
                "selected checkpoint manifest exceeds the exact-restore profile bound",
            ));
        }
        let manifest_value = super::super::gateway_json::parse(&manifest)
            .map_err(|_| String::from("selected checkpoint manifest is invalid JSON"))?;
        let manifest_digest = format!("sha256:{}", sts2_harness::sha256_hex(&manifest));
        let exact_state_digest = manifest_value["exact_state_digest"]
            .as_str()
            .ok_or_else(|| String::from("selected checkpoint manifest omitted exact state"))?
            .to_owned();
        validate_checkpoint_identity(
            selected,
            checkpoint,
            &checkpoint_id,
            &manifest_value,
            &exact_state_digest,
        )?;
        let reference = ExactCheckpointReference {
            exact_state_digest: ExactStateDigest::parse(&exact_state_digest)
                .map_err(|_| String::from("selected exact-state digest is invalid"))?,
            exact_checkpoint_id: checkpoint_id.clone(),
            boundary_kind: required_manifest_string(&manifest_value, &["boundary", "kind"])?,
            boundary_phase: required_manifest_string(&manifest_value, &["boundary", "phase"])?,
            assurance: ExactAssurance::CaptureOnly,
        };
        verify_checkpoint(&store, &reference, &pins)?;
        let references = manifest_value["restore_artifacts"]
            .as_array()
            .ok_or_else(|| String::from("selected checkpoint restore references are invalid"))?;
        let artifact_reference_count = references
            .len()
            .checked_add(1)
            .ok_or_else(|| String::from("exact-restore reference count overflowed"))?;
        if artifact_reference_count > MAX_ARTIFACT_REFERENCES {
            return Err(String::from(
                "selected checkpoint exceeds the 64-reference exact-restore profile bound",
            ));
        }
        validate_restore_references(restore_closure, references)?;
        let canonical_payload = &manifest_value["canonical_payload"];
        let artifact_references = protocol_references(canonical_payload, references)?;
        let canonical_digest = required_manifest_string(canonical_payload, &["digest"])?;
        let canonical_bytes = read_verified_blob(&store, &canonical_digest, canonical_payload)?;
        let restore_bytes = references
            .iter()
            .map(|entry| {
                let digest = required_manifest_string(entry, &["digest"])?;
                Ok((digest.clone(), read_verified_blob(&store, &digest, entry)?))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let (distinct_payloads, aggregate, distinct_blob_count) = distinct_payloads(
            manifest.len(),
            &canonical_digest,
            &canonical_bytes,
            &restore_bytes,
        )?;
        if aggregate > MAX_CLOSURE_BYTES as u64 {
            return Err(String::from(
                "selected checkpoint exceeds the 64 MiB exact-restore profile bound",
            ));
        }
        let mut transfer_blobs = Vec::with_capacity(distinct_payloads.len() + 1);
        transfer_blobs.push(TransferBlob {
            digest: manifest_digest.clone(),
            bytes: manifest.clone(),
        });
        transfer_blobs.extend(
            distinct_payloads
                .into_iter()
                .filter(|(digest, _)| digest != &manifest_digest)
                .map(|(digest, bytes)| TransferBlob { digest, bytes }),
        );
        let boundary = json!({
            "kind": reference.boundary_kind,
            "phase": reference.boundary_phase,
            "game_tick": manifest_value["boundary"]["game_tick"],
        });
        if !boundary["game_tick"].is_u64() {
            return Err(String::from(
                "exact-restore requires the checkpoint's recorded game_tick",
            ));
        }
        Ok(Self {
            manifest_digest,
            manifest_size_bytes: u64::try_from(manifest.len())
                .map_err(|_| String::from("manifest size overflowed"))?,
            closure_digest: closure_digest(&manifest, &canonical_bytes, &restore_bytes)?,
            exact_state_digest,
            checkpoint_id: checkpoint_id.as_str().to_owned(),
            boundary,
            artifact_references,
            transfer_blobs,
            artifact_reference_count,
            distinct_blob_count,
            aggregate_closure_bytes: aggregate,
            compatibility_digest: pins.compatibility_digest,
            coverage_contract_digest: pins.coverage_contract_digest,
        })
    }

    pub(super) fn begin_payload(
        &self,
        selected: &SelectedBranchContinuation,
        operation_id: &str,
        expected_owner: Value,
    ) -> Result<Value, String> {
        let branch = selected.branch();
        let episode_id = branch
            .episode_id
            .as_deref()
            .ok_or_else(|| String::from("selected branch has no episode identity"))?;
        let trajectory_id = branch
            .trajectory_id
            .as_deref()
            .ok_or_else(|| String::from("selected branch has no trajectory identity"))?;
        Ok(json!({
            "operation_id": operation_id,
            "expected_owner": expected_owner,
            "branch": {
                "experiment_id": branch.experiment_id,
                "branch_id": branch.branch_id,
                "metadata_revision": branch.metadata_revision,
                "run_id": branch.run_id,
                "episode_id": episode_id,
                "trajectory_id": trajectory_id,
            },
            "checkpoint_id": self.checkpoint_id,
            "exact_state_digest": self.exact_state_digest,
            "manifest_digest": self.manifest_digest,
            "manifest_size_bytes": self.manifest_size_bytes,
            "closure_digest": self.closure_digest,
            "compatibility_digest": self.compatibility_digest,
            "coverage_contract_digest": self.coverage_contract_digest,
            "boundary": self.boundary,
            "artifacts": self.artifact_references,
            "artifact_reference_count": self.artifact_reference_count,
            "distinct_blob_count": self.distinct_blob_count,
            "aggregate_closure_bytes": self.aggregate_closure_bytes,
        }))
    }

    #[cfg(test)]
    pub(crate) fn begin_payload_for_test(
        &self,
        selected: &SelectedBranchContinuation,
        operation_id: &str,
        expected_owner: Value,
    ) -> Result<Value, String> {
        self.begin_payload(selected, operation_id, expected_owner)
    }
}

fn verify_checkpoint(
    store: &ExactArtifactStore,
    reference: &ExactCheckpointReference,
    pins: &ProfilePins,
) -> Result<(), String> {
    match sts2_harness::verify_checkpoint(
        store,
        reference,
        &pins.compatibility_digest,
        &pins.coverage_contract_digest,
    ) {
        VerificationOutcome::Verified(_) => Ok(()),
        VerificationOutcome::Rejected(failure) => Err(format!(
            "selected checkpoint failed exact-restore verification: {}",
            failure.as_str()
        )),
    }
}

type DistinctPayloads = (BTreeMap<String, Vec<u8>>, u64, usize);

fn distinct_payloads(
    manifest_size: usize,
    canonical_digest: &str,
    canonical_bytes: &[u8],
    restore_bytes: &[(String, Vec<u8>)],
) -> Result<DistinctPayloads, String> {
    let mut payloads = BTreeMap::from([(canonical_digest.to_owned(), canonical_bytes.to_vec())]);
    for (digest, bytes) in restore_bytes {
        if let Some(prior) = payloads.get(digest) {
            if prior != bytes {
                return Err(String::from(
                    "aliased exact-restore references disagree on blob bytes",
                ));
            }
        } else {
            payloads.insert(digest.clone(), bytes.clone());
        }
    }
    let mut aggregate = u64::try_from(manifest_size)
        .map_err(|_| String::from("manifest size exceeds the exact-restore bound"))?;
    for bytes in payloads.values() {
        aggregate = aggregate
            .checked_add(
                u64::try_from(bytes.len())
                    .map_err(|_| String::from("artifact size exceeds the exact-restore bound"))?,
            )
            .ok_or_else(|| String::from("aggregate exact-restore size overflowed"))?;
    }
    let count = payloads.len();
    Ok((payloads, aggregate, count))
}

fn closure_digest(
    manifest: &[u8],
    canonical_payload: &[u8],
    restore_artifacts: &[(String, Vec<u8>)],
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(CLOSURE_DOMAIN);
    update_length_prefixed(&mut hasher, manifest)?;
    update_length_prefixed(&mut hasher, canonical_payload)?;
    let canonical_digest = format!("sha256:{}", sts2_harness::sha256_hex(canonical_payload));
    let mut included = BTreeSet::from([canonical_digest]);
    for (digest, bytes) in restore_artifacts {
        if included.insert(digest.clone()) {
            update_length_prefixed(&mut hasher, bytes)?;
        }
    }
    Ok(format!(
        "sha256:{}",
        hex_digest(hasher.finalize().as_slice())
    ))
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let length = u64::try_from(bytes.len())
        .map_err(|_| String::from("exact-restore closure item length overflowed"))?;
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
