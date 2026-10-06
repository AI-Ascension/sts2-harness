// SPDX-License-Identifier: MIT

use super::contract::{TargetAdmissionBinding, validate_identifier};
use super::contract_seed_v2::WorkflowRunRequestV2;
use super::seed_key::{SeedKeyError, hex_lower};

#[path = "seed_v2_crypto_encoding.rs"]
mod encoding;
use encoding::{hash_framed, parse_digest};
pub(super) use encoding::{validate_candidate_record, validate_operation_record};
#[path = "seed_v2_crypto_records.rs"]
mod records;
pub(super) use records::{
    candidate_record, derive_candidate_from_operation, load_pinned_operation_key,
    prepare_seed_operation, verify_candidate, verify_stored_record,
};

const REQUEST_DIGEST_DOMAIN: &str = "ascension.workflow-run-request/v2";
const ACTOR_DIGEST_DOMAIN: &str = "ascension.workflow-seed-actor/v1";
const CONFIGURATION_DIGEST_DOMAIN: &str = "ascension.seed-admitted-configuration/v1";
const OPERATION_ID_DOMAIN: &str = "ascension.seed-operation-id/v2";
const DERIVATION_DOMAIN: &str = "ascension.seed-derive-once/v2";

pub(super) fn request_digest(
    request: &WorkflowRunRequestV2,
    subject: &str,
) -> Result<String, SeedKeyError> {
    validate_identifier("subject", subject).map_err(|_| SeedKeyError::InvalidIdentity)?;
    let serialized = serde_json::to_vec(request).map_err(|_| SeedKeyError::InvalidIdentity)?;
    hash_framed(&[
        REQUEST_DIGEST_DOMAIN.as_bytes(),
        subject.as_bytes(),
        &serialized,
    ])
}

pub(super) fn actor_digest(subject: &str) -> Result<String, SeedKeyError> {
    validate_identifier("subject", subject).map_err(|_| SeedKeyError::InvalidIdentity)?;
    hash_framed(&[ACTOR_DIGEST_DOMAIN.as_bytes(), subject.as_bytes()])
}

pub(super) fn configuration_digest(
    binding: &TargetAdmissionBinding,
) -> Result<String, SeedKeyError> {
    let serialized = serde_json::to_vec(binding).map_err(|_| SeedKeyError::InvalidIdentity)?;
    hash_framed(&[CONFIGURATION_DIGEST_DOMAIN.as_bytes(), &serialized])
}

pub(super) fn operation_id(
    request_id: &str,
    workflow_run_id: &str,
    request_digest: &str,
) -> Result<String, SeedKeyError> {
    let digest = hash_framed(&[
        OPERATION_ID_DOMAIN.as_bytes(),
        request_id.as_bytes(),
        workflow_run_id.as_bytes(),
        request_digest.as_bytes(),
    ])?;
    let bytes = parse_digest(&digest)?;
    let mut operation_id = String::from("seedop.v2.");
    operation_id.push_str(&hex_lower(&bytes[..16]));
    Ok(operation_id)
}

#[cfg(test)]
#[path = "seed_v2_crypto_tests.rs"]
mod tests;
