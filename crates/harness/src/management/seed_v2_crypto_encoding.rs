// SPDX-License-Identifier: MIT

use sha2::{Digest, Sha256};

use super::super::contract::validate_identifier;
use super::super::contract_seed_v2::{
    SEED_DERIVATION_ALGORITHM_V1, SeedModeV2, SeedOperationPhaseV2, StoredSeedBindingV2,
    StoredSeedOperationV2, WORKFLOW_SEED_BINDING_V2_SCHEMA, WORKFLOW_SEED_OPERATION_V2_SCHEMA,
};
use super::super::seed_key::{SeedKeyError, SeedKeyHandle, framed, hex_lower};
use super::{DERIVATION_DOMAIN, configuration_digest, operation_id};
use crate::seed_binding::canonicalize_seed;

pub(super) fn hash_framed(parts: &[&[u8]]) -> Result<String, SeedKeyError> {
    let mut digest = Sha256::new();
    for part in parts {
        let length = u32::try_from(part.len()).map_err(|_| SeedKeyError::MessageTooLarge)?;
        digest.update(length.to_be_bytes());
        digest.update(part);
    }
    Ok(hex_lower(&digest.finalize()))
}

pub(super) fn parse_digest(value: &str) -> Result<[u8; 32], SeedKeyError> {
    if value.len() != 64 {
        return Err(SeedKeyError::InvalidIdentity);
    }
    let mut bytes = [0_u8; 32];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let high = nibble(value.as_bytes()[index * 2]).ok_or(SeedKeyError::InvalidIdentity)?;
        let low = nibble(value.as_bytes()[index * 2 + 1]).ok_or(SeedKeyError::InvalidIdentity)?;
        *slot = (high << 4) | low;
    }
    Ok(bytes)
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

pub(in crate::management) fn validate_operation_record(
    record: &StoredSeedOperationV2,
) -> Result<(), SeedKeyError> {
    if record.schema_version != WORKFLOW_SEED_OPERATION_V2_SCHEMA
        || record.mode != SeedModeV2::DeriveOnce
        || record.derivation.algorithm_id != SEED_DERIVATION_ALGORITHM_V1
        || record.actor_digest.len() != 64
        || record.request_digest.len() != 64
        || record.configuration_digest.len() != 64
        || !matches!(
            record.phase,
            SeedOperationPhaseV2::KeyPinned | SeedOperationPhaseV2::CandidatePersisted
        )
    {
        return Err(SeedKeyError::InvalidIdentity);
    }
    for identifier in [
        &record.request_id,
        &record.workflow_run_id,
        &record.operation_id,
    ] {
        validate_identifier("seed_operation_identity", identifier)
            .map_err(|_| SeedKeyError::InvalidIdentity)?;
    }
    for digest in [
        &record.actor_digest,
        &record.request_digest,
        &record.configuration_digest,
    ] {
        if !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(SeedKeyError::InvalidIdentity);
        }
    }
    record
        .derivation
        .key
        .validate()
        .map_err(|_| SeedKeyError::InvalidIdentity)?;
    record
        .admitted_configuration
        .validate()
        .map_err(|_| SeedKeyError::InvalidIdentity)?;
    if configuration_digest(&record.admitted_configuration)? != record.configuration_digest
        || operation_id(
            &record.request_id,
            &record.workflow_run_id,
            &record.request_digest,
        )? != record.operation_id
    {
        return Err(SeedKeyError::InvalidIdentity);
    }
    Ok(())
}

pub(super) fn derivation_transcript(
    key: &SeedKeyHandle,
    subject: &str,
    request_digest: &str,
    workflow_run_id: &str,
    operation_id: &str,
    configuration_digest: &str,
) -> Result<Vec<u8>, SeedKeyError> {
    framed(&[
        DERIVATION_DOMAIN.as_bytes(),
        SEED_DERIVATION_ALGORITHM_V1.as_bytes(),
        key.identity().authority_id.as_bytes(),
        key.identity().version.as_bytes(),
        subject.as_bytes(),
        request_digest.as_bytes(),
        workflow_run_id.as_bytes(),
        operation_id.as_bytes(),
        configuration_digest.as_bytes(),
    ])
}

pub(in crate::management) fn validate_candidate_record(
    record: &StoredSeedBindingV2,
) -> Result<(), SeedKeyError> {
    if record.schema_version != WORKFLOW_SEED_BINDING_V2_SCHEMA
        || record.request_digest.len() != 64
        || record.actor_digest.len() != 64
        || record.configuration_digest.len() != 64
    {
        return Err(SeedKeyError::InvalidIdentity);
    }
    record
        .admitted_configuration
        .validate()
        .map_err(|_| SeedKeyError::InvalidIdentity)?;
    if configuration_digest(&record.admitted_configuration)? != record.configuration_digest {
        return Err(SeedKeyError::InvalidIdentity);
    }
    for identifier in [
        &record.request_id,
        &record.workflow_run_id,
        &record.operation_id,
    ] {
        validate_identifier("seed_record_identity", identifier)
            .map_err(|_| SeedKeyError::InvalidIdentity)?;
    }
    for digest in [
        &record.request_digest,
        &record.actor_digest,
        &record.configuration_digest,
    ] {
        if !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(SeedKeyError::InvalidIdentity);
        }
    }
    let canonical =
        canonicalize_seed(&record.effective_seed).map_err(|_| SeedKeyError::InvalidIdentity)?;
    if canonical != record.effective_seed {
        return Err(SeedKeyError::InvalidIdentity);
    }
    match (
        record.mode,
        record.requested_seed.as_deref(),
        record.derivation.as_ref(),
    ) {
        (SeedModeV2::Explicit, Some(requested), None) if requested == record.effective_seed => {}
        (SeedModeV2::DeriveOnce, None, Some(pinned))
            if pinned.algorithm_id == SEED_DERIVATION_ALGORITHM_V1
                && record.effective_seed.len() == 32
                && record
                    .effective_seed
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) =>
        {
            pinned.key.validate()?;
        }
        _ => return Err(SeedKeyError::InvalidIdentity),
    }
    Ok(())
}
