// SPDX-License-Identifier: MIT

use super::auth::AuthContext;
use super::contract::validate_identifier;
use super::contract_seed_v2::{
    SEED_DERIVATION_ALGORITHM_V1, SeedModeV2, StoredSeedBindingV2, StoredSeedDerivationV2,
    WORKFLOW_SEED_BINDING_V2_SCHEMA, WorkflowRunRequestV2,
};
use super::contract_target_admission::TargetAdmissionBinding;
use super::seed_key::{
    SeedDerivationKeyAuthority, SeedKeyError, SeedKeyHandle, derive_seed, framed, hex_lower,
};
use crate::seed_binding::canonicalize_seed;

#[path = "seed_v2_crypto_encoding.rs"]
mod encoding;
use encoding::{hash_framed, parse_digest};

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

pub(super) fn candidate_record(
    request: &WorkflowRunRequestV2,
    actor: &AuthContext,
    request_digest: &str,
    workflow_run_id: &str,
    binding: &TargetAdmissionBinding,
    key_authority: Option<&dyn SeedDerivationKeyAuthority>,
) -> Result<StoredSeedBindingV2, SeedKeyError> {
    let operation_id = operation_id(&request.request_id, workflow_run_id, request_digest)?;
    let configuration_digest = configuration_digest(binding)?;
    let actor_digest = actor_digest(&actor.subject)?;
    let (requested_seed, effective_seed, derivation) = match request.seed.mode {
        SeedModeV2::Explicit => {
            let requested = request
                .seed
                .seed
                .as_deref()
                .ok_or(SeedKeyError::InvalidIdentity)?;
            let seed = canonicalize_seed(requested).map_err(|_| SeedKeyError::InvalidIdentity)?;
            (Some(seed.clone()), seed, None)
        }
        SeedModeV2::DeriveOnce => {
            let authority = key_authority.ok_or(SeedKeyError::Unavailable)?;
            let key = authority.current_key()?;
            key.identity().validate()?;
            if !key.verifies_identity(key.identity())? {
                return Err(SeedKeyError::InvalidKeyMaterial);
            }
            let transcript = derivation_transcript(
                &key,
                &actor.subject,
                request_digest,
                workflow_run_id,
                &operation_id,
                &configuration_digest,
            )?;
            let seed = derive_seed(&key, &transcript)?;
            (
                None,
                seed,
                Some(StoredSeedDerivationV2 {
                    algorithm_id: SEED_DERIVATION_ALGORITHM_V1.to_owned(),
                    key: key.identity().clone(),
                }),
            )
        }
    };
    let record = StoredSeedBindingV2 {
        schema_version: WORKFLOW_SEED_BINDING_V2_SCHEMA.to_owned(),
        request_id: request.request_id.clone(),
        actor_digest,
        request_digest: request_digest.to_owned(),
        workflow_run_id: workflow_run_id.to_owned(),
        operation_id,
        mode: request.seed.mode,
        requested_seed,
        effective_seed,
        derivation,
        admitted_configuration: binding.clone(),
        configuration_digest,
        state: super::contract_seed_v2::SeedBindingStateV2::CandidatePersisted,
    };
    validate_candidate_record(&record)?;
    Ok(record)
}

pub(super) fn verify_candidate(
    record: &StoredSeedBindingV2,
    request: &WorkflowRunRequestV2,
    actor: &AuthContext,
    request_digest: &str,
    workflow_run_id: &str,
    key_authority: Option<&dyn SeedDerivationKeyAuthority>,
) -> Result<bool, SeedKeyError> {
    validate_candidate_record(record)?;
    if record.request_id != request.request_id
        || record.request_digest != request_digest
        || record.workflow_run_id != workflow_run_id
        || record.mode != request.seed.mode
        || record.actor_digest != actor_digest(&actor.subject)?
        || record.operation_id
            != operation_id(&request.request_id, workflow_run_id, request_digest)?
    {
        return Ok(false);
    }
    match (record.mode, record.derivation.as_ref()) {
        (SeedModeV2::Explicit, None) => {
            let requested = request
                .seed
                .seed
                .as_deref()
                .ok_or(SeedKeyError::InvalidIdentity)?;
            let requested =
                canonicalize_seed(requested).map_err(|_| SeedKeyError::InvalidIdentity)?;
            Ok(record.requested_seed.as_deref() == Some(requested.as_str())
                && record.effective_seed == requested)
        }
        (SeedModeV2::DeriveOnce, Some(pinned)) => {
            if request.seed.seed.is_some() || pinned.algorithm_id != SEED_DERIVATION_ALGORITHM_V1 {
                return Ok(false);
            }
            let authority = key_authority.ok_or(SeedKeyError::Unavailable)?;
            let Some(key) = authority.key_for(&pinned.key.authority_id, &pinned.key.version)?
            else {
                return Err(SeedKeyError::KeyVersionUnavailable);
            };
            if !key.verifies_identity(&pinned.key)? {
                return Err(SeedKeyError::InvalidKeyMaterial);
            }
            let transcript = derivation_transcript(
                &key,
                &actor.subject,
                request_digest,
                workflow_run_id,
                &record.operation_id,
                &record.configuration_digest,
            )?;
            Ok(derive_seed(&key, &transcript)? == record.effective_seed)
        }
        _ => Ok(false),
    }
}

pub(super) fn verify_stored_record(
    record: &StoredSeedBindingV2,
    actor: &AuthContext,
    key_authority: Option<&dyn SeedDerivationKeyAuthority>,
) -> Result<bool, SeedKeyError> {
    validate_candidate_record(record)?;
    if record.actor_digest != actor_digest(&actor.subject)?
        || record.operation_id
            != operation_id(
                &record.request_id,
                &record.workflow_run_id,
                &record.request_digest,
            )?
    {
        return Ok(false);
    }
    match (record.mode, record.derivation.as_ref()) {
        (SeedModeV2::Explicit, None) => {
            Ok(record.requested_seed.as_deref() == Some(record.effective_seed.as_str()))
        }
        (SeedModeV2::DeriveOnce, Some(pinned)) => {
            if pinned.algorithm_id != SEED_DERIVATION_ALGORITHM_V1 {
                return Ok(false);
            }
            let authority = key_authority.ok_or(SeedKeyError::Unavailable)?;
            let Some(key) = authority.key_for(&pinned.key.authority_id, &pinned.key.version)?
            else {
                return Err(SeedKeyError::KeyVersionUnavailable);
            };
            if !key.verifies_identity(&pinned.key)? {
                return Err(SeedKeyError::InvalidKeyMaterial);
            }
            let transcript = derivation_transcript(
                &key,
                &actor.subject,
                &record.request_digest,
                &record.workflow_run_id,
                &record.operation_id,
                &record.configuration_digest,
            )?;
            Ok(derive_seed(&key, &transcript)? == record.effective_seed)
        }
        _ => Ok(false),
    }
}

fn derivation_transcript(
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

pub(super) fn validate_candidate_record(record: &StoredSeedBindingV2) -> Result<(), SeedKeyError> {
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

#[cfg(test)]
#[path = "seed_v2_crypto_tests.rs"]
mod tests;
