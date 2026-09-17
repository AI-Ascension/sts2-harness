// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::{Value, json};
use sts2_harness::{BranchArtifactRole, ExactArtifactStore, ExactCheckpointId};

use super::super::MAX_BLOB_BYTES;
use crate::runtime_support::branch_continuation_runtime::SelectedBranchContinuation;

pub(super) fn validate_checkpoint_identity(
    selected: &SelectedBranchContinuation,
    checkpoint: &sts2_harness::BranchArtifactReference,
    checkpoint_id: &ExactCheckpointId,
    manifest: &Value,
    exact_state_digest: &str,
) -> Result<(), String> {
    if checkpoint_id.as_str() != checkpoint.artifact_id
        || selected.branch().fork.state_digest.as_str() != exact_state_digest
        || Some(selected.branch().boundary.as_str()) != manifest["boundary"]["kind"].as_str()
    {
        return Err(String::from(
            "selected checkpoint does not match the durable branch state and boundary",
        ));
    }
    Ok(())
}

pub(super) fn validate_restore_references(
    restore_closure: &[sts2_harness::BranchArtifactReference],
    references: &[Value],
) -> Result<(), String> {
    let retained = restore_closure
        .iter()
        .map(|item| {
            if item.role != BranchArtifactRole::RestoreClosure {
                return Err(String::from(
                    "selected branch restore closure contains an unexpected artifact role",
                ));
            }
            item.artifact_id
                .strip_prefix("sha256:")
                .map(|digest| format!("sha256:{digest}"))
                .ok_or_else(|| String::from("selected restore artifact digest is invalid"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let manifest = references
        .iter()
        .map(|entry| {
            entry["digest"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| String::from("checkpoint restore digest is invalid"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if retained != manifest {
        return Err(String::from(
            "selected branch restore references differ from the checkpoint manifest",
        ));
    }
    Ok(())
}

pub(super) fn protocol_references(
    canonical: &Value,
    references: &[Value],
) -> Result<Vec<Value>, String> {
    let mut output = Vec::with_capacity(references.len() + 1);
    output.push(protocol_artifact_reference(canonical, true)?);
    output.extend(
        references
            .iter()
            .map(|entry| protocol_artifact_reference(entry, false))
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(output)
}

fn protocol_artifact_reference(entry: &Value, canonical: bool) -> Result<Value, String> {
    let role = if canonical {
        String::from("canonical-state")
    } else {
        required_manifest_string(entry, &["role"])?
    };
    let digest = required_manifest_string(entry, &["digest"])?;
    let codec = required_manifest_string(entry, &["codec"])?;
    let size_bytes = entry["size_bytes"]
        .as_u64()
        .ok_or_else(|| String::from("checkpoint artifact size is invalid"))?;
    if size_bytes > MAX_BLOB_BYTES as u64 {
        return Err(String::from(
            "checkpoint artifact exceeds the 16 MiB exact-restore profile bound",
        ));
    }
    Ok(json!({"role": role, "digest": digest, "size_bytes": size_bytes, "codec": codec}))
}

pub(super) fn read_verified_blob(
    store: &ExactArtifactStore,
    digest: &str,
    entry: &Value,
) -> Result<Vec<u8>, String> {
    let blob_digest = sts2_harness::BlobDigest::parse(digest)
        .map_err(|_| String::from("checkpoint artifact digest is invalid"))?;
    let bytes = store
        .read_blob(&blob_digest)
        .map_err(|error| format!("checkpoint artifact is unavailable: {error}"))?;
    if bytes.len() > MAX_BLOB_BYTES
        || entry["size_bytes"].as_u64() != u64::try_from(bytes.len()).ok()
    {
        return Err(String::from(
            "checkpoint artifact size differs from its manifest reference",
        ));
    }
    Ok(bytes)
}

pub(super) fn required_manifest_string(value: &Value, path: &[&str]) -> Result<String, String> {
    let mut current = value;
    for member in path {
        current = &current[*member];
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("checkpoint manifest omitted {}", path.join(".")))
}
