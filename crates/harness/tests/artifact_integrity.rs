// SPDX-License-Identifier: MIT

use std::error::Error;

use sha2::{Digest as _, Sha256};
use sts2_harness::{
    ArtifactDraft, ArtifactId, ArtifactKind, ArtifactLineage, ArtifactMetadata,
    ArtifactMetadataInput, Digest, RunId, SchemaVersion,
};

fn metadata(bytes: &[u8]) -> Result<ArtifactMetadata, Box<dyn Error>> {
    let run = RunId::new(1).ok_or("invalid run")?;
    Ok(ArtifactMetadata::from_input(
        ArtifactMetadataInput {
            artifact_id: ArtifactId::new(1).ok_or("invalid artifact")?,
            owner_run: run,
            kind: ArtifactKind::Trajectory,
            schema_version: SchemaVersion::new(1).ok_or("invalid schema")?,
            content_digest: Digest::new(format!("{:x}", Sha256::digest(bytes)))?,
            producer: "synthetic-test".to_owned(),
            lineage: ArtifactLineage::new(run, None, Vec::new())?,
        },
        bytes.len() as u64,
    )?)
}

#[test]
fn exact_digest_is_required_even_when_lengths_match() -> Result<(), Box<dyn Error>> {
    let original = b"original";
    let metadata = metadata(original)?;
    assert!(ArtifactDraft::new(metadata.clone(), original.to_vec()).is_ok());
    let error = ArtifactDraft::new(metadata, b"modified".to_vec())
        .err()
        .ok_or("same-length corrupted content was accepted")?;
    assert_eq!(error.code(), "artifact_digest_mismatch");
    assert!(!error.is_retryable());
    Ok(())
}

#[test]
fn empty_content_uses_the_real_empty_digest() -> Result<(), Box<dyn Error>> {
    let metadata = metadata(b"")?;
    assert_eq!(
        metadata.content_digest().as_str(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert!(ArtifactDraft::new(metadata, Vec::new()).is_ok());
    Ok(())
}
