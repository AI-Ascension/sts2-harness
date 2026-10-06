// SPDX-License-Identifier: MIT

// Versioned request, receipt, and read projections for immutable draft publication.

pub const CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-draft-publication-request.v1";
pub const CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-draft-publication-receipt.v1";
pub const CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-draft-publication-lookup.v1";
pub const CONTEXT_OWNER_PUBLISHED_SOURCES_VIEW_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-published-sources.v1";

/// CAS request to publish the exact current draft as immutable owner source bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerDraftPublicationRequest {
    pub schema_version: String,
    pub request_id: String,
    pub draft_id: String,
    pub expected_draft_version: u64,
    pub expected_owner_state_version: u64,
    pub expected_base_revision_id: String,
    pub expected_binding_id: String,
    pub expected_binding_digest: String,
    pub expected_boundary: ContextBoundary,
}

impl ContextOwnerDraftPublicationRequest {
    pub(crate) fn validate(&self) -> Result<(), ManagementError> {
        if self.schema_version != CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION
            || self.expected_draft_version == 0
            || self.expected_owner_state_version == 0
        {
            return Err(ManagementError::invalid(
                "context_publication_request_schema",
                "publication request schema or expected version is invalid",
            ));
        }
        validate_identifier("context_request_id", &self.request_id)?;
        validate_identifier("context_draft_id", &self.draft_id)?;
        validate_identifier("context_revision_id", &self.expected_base_revision_id)?;
        validate_identifier("context_binding_id", &self.expected_binding_id)?;
        validate_digest("context_binding_digest", &self.expected_binding_digest)?;
        validate_boundary(&self.expected_boundary)?;
        Ok(())
    }

    pub(crate) fn digest(&self) -> Result<String, ManagementError> {
        self.validate()?;
        let boundary = &self.expected_boundary;
        let mut frame = b"ascension.context-control.draft-publication.request-digest.v1\0".to_vec();
        append_lp(&mut frame, &self.schema_version)?;
        append_lp(&mut frame, &self.request_id)?;
        append_lp(&mut frame, &self.draft_id)?;
        append_u64(&mut frame, self.expected_draft_version);
        append_u64(&mut frame, self.expected_owner_state_version);
        append_lp(&mut frame, &self.expected_base_revision_id)?;
        append_lp(&mut frame, &self.expected_binding_id)?;
        append_digest(&mut frame, &self.expected_binding_digest)?;
        append_lp(&mut frame, &boundary.run_id)?;
        append_lp(&mut frame, &boundary.episode_id)?;
        append_lp(&mut frame, &boundary.agent_id)?;
        append_lp(&mut frame, &boundary.state_id)?;
        append_u64(&mut frame, boundary.generation);
        append_digest(&mut frame, &boundary.observation_sha256)?;
        append_digest(&mut frame, &boundary.catalog_sha256)?;
        append_lp(&mut frame, &boundary.adapter_revision)?;
        append_lp(&mut frame, &boundary.model_revision)?;
        append_digest(&mut frame, &boundary.configuration_sha256)?;
        append_digest(&mut frame, &boundary.output_schema_sha256)?;
        append_u64(&mut frame, boundary.controller_epoch);
        append_u64(&mut frame, boundary.gate_epoch);
        append_u64(&mut frame, boundary.control_version);
        Ok(sha256_hex(frame))
    }
}

/// Immutable receipt encrypted by the run-scoped context-control store.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerDraftPublicationReceipt {
    pub schema_version: String,
    pub owner_id: String,
    pub workflow_run_id: String,
    pub actor_subject: String,
    pub binding: ContextOwnerBinding,
    pub boundary: ContextBoundary,
    pub request_id: String,
    pub request_digest: String,
    pub draft_id: String,
    pub draft_version: u64,
    pub base_revision_id: String,
    pub expected_owner_state_version: u64,
    pub resulting_owner_state_version: u64,
    pub source_id: String,
    pub source_version: u64,
    pub source_digest: String,
    pub published_at: u64,
    pub expires_at: u64,
}

/// Authenticated current run metadata. Entries contain source identities only; content,
/// actor subjects, request IDs, and encrypted receipt bodies stay behind the owner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerPublishedSourcesView {
    pub schema_version: String,
    pub owner_id: String,
    pub workflow_run_id: String,
    pub definition_digest: String,
    pub instance_id: String,
    pub binding: ContextOwnerBinding,
    pub boundary: ContextBoundary,
    pub owner_state_version: u64,
    pub active_source: Option<ContextBindingSource>,
    pub publications: Vec<ContextBindingSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerDraftPublicationLookupRequest {
    pub schema_version: String,
    pub request: ContextOwnerDraftPublicationRequest,
}

fn append_lp(frame: &mut Vec<u8>, value: &str) -> Result<(), ManagementError> {
    let bytes = value.as_bytes();
    append_u64(
        frame,
        u64::try_from(bytes.len()).map_err(|_| {
            ManagementError::invalid("context_publication_request_size", "field is too large")
        })?,
    );
    frame.extend_from_slice(bytes);
    Ok(())
}

fn append_u64(frame: &mut Vec<u8>, value: u64) {
    frame.extend_from_slice(&value.to_be_bytes());
}

fn append_digest(frame: &mut Vec<u8>, value: &str) -> Result<(), ManagementError> {
    validate_digest("context_publication_digest", value)?;
    for pair in value.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0]);
        let low = hex_nibble(pair[1]);
        let byte = high
            .and_then(|high| low.map(|low| high << 4 | low))
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_publication_digest",
                    "digest is not lowercase hexadecimal",
                )
            })?;
        frame.push(byte);
    }
    Ok(())
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_control::ContextBoundary;

    #[test]
    fn publication_request_digest_matches_binary_golden_frame() {
        let request = ContextOwnerDraftPublicationRequest {
            schema_version: CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION.to_owned(),
            request_id: "request-test-0001".to_owned(),
            draft_id: "draft-test-0002".to_owned(),
            expected_draft_version: 7,
            expected_owner_state_version: 11,
            expected_base_revision_id: "revision-4".to_owned(),
            expected_binding_id: "binding-test".to_owned(),
            expected_binding_digest:
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_owned(),
            expected_boundary: ContextBoundary {
                run_id: "run-test".to_owned(),
                episode_id: "episode-test".to_owned(),
                agent_id: "agent-test".to_owned(),
                state_id: "state-test".to_owned(),
                generation: 42,
                observation_sha256:
                    "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f".to_owned(),
                catalog_sha256: "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f"
                    .to_owned(),
                adapter_revision: "adapter.v1".to_owned(),
                model_revision: "model.v2".to_owned(),
                configuration_sha256:
                    "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f".to_owned(),
                output_schema_sha256:
                    "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f".to_owned(),
                controller_epoch: 3,
                gate_epoch: 5,
                control_version: 9,
            },
        };
        let mut frame = b"ascension.context-control.draft-publication.request-digest.v1\0".to_vec();
        append_test_lp(&mut frame, &request.schema_version);
        append_test_lp(&mut frame, &request.request_id);
        append_test_lp(&mut frame, &request.draft_id);
        append_test_u64(&mut frame, request.expected_draft_version);
        append_test_u64(&mut frame, request.expected_owner_state_version);
        append_test_lp(&mut frame, &request.expected_base_revision_id);
        append_test_lp(&mut frame, &request.expected_binding_id);
        append_test_digest(&mut frame, &request.expected_binding_digest);
        let boundary = &request.expected_boundary;
        append_test_lp(&mut frame, &boundary.run_id);
        append_test_lp(&mut frame, &boundary.episode_id);
        append_test_lp(&mut frame, &boundary.agent_id);
        append_test_lp(&mut frame, &boundary.state_id);
        append_test_u64(&mut frame, boundary.generation);
        append_test_digest(&mut frame, &boundary.observation_sha256);
        append_test_digest(&mut frame, &boundary.catalog_sha256);
        append_test_lp(&mut frame, &boundary.adapter_revision);
        append_test_lp(&mut frame, &boundary.model_revision);
        append_test_digest(&mut frame, &boundary.configuration_sha256);
        append_test_digest(&mut frame, &boundary.output_schema_sha256);
        append_test_u64(&mut frame, boundary.controller_epoch);
        append_test_u64(&mut frame, boundary.gate_epoch);
        append_test_u64(&mut frame, boundary.control_version);

        let expected = "d723f9d3f5cea47713025ffaf09889201f22dc8305a8a69f2646f88cf4b1333c";
        assert_eq!(frame.len(), 530);
        assert_eq!(crate::sha256_hex(&frame), expected);
        assert_eq!(request.digest().expect("request digest"), expected);
    }

    fn append_test_lp(frame: &mut Vec<u8>, value: &str) {
        append_test_u64(frame, value.len() as u64);
        frame.extend_from_slice(value.as_bytes());
    }

    fn append_test_u64(frame: &mut Vec<u8>, value: u64) {
        frame.extend_from_slice(&value.to_be_bytes());
    }

    fn append_test_digest(frame: &mut Vec<u8>, value: &str) {
        for pair in value.as_bytes().chunks_exact(2) {
            let high = u8::from_str_radix(std::str::from_utf8(&pair[..1]).expect("hex"), 16)
                .expect("hex digit");
            let low = u8::from_str_radix(std::str::from_utf8(&pair[1..]).expect("hex"), 16)
                .expect("hex digit");
            frame.push(high << 4 | low);
        }
    }
}
