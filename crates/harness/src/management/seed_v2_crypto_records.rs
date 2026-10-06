// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::contract::TargetAdmissionBinding;
use super::super::contract_seed_v2::{
    SEED_DERIVATION_ALGORITHM_V1, SeedBindingStateV2, SeedModeV2, SeedOperationPhaseV2,
    StoredSeedBindingV2, StoredSeedDerivationV2, StoredSeedOperationV2,
    WORKFLOW_SEED_BINDING_V2_SCHEMA, WORKFLOW_SEED_OPERATION_V2_SCHEMA, WorkflowRunRequestV2,
};
use super::super::seed_key::{
    SeedDerivationKeyAuthority, SeedKeyError, SeedKeyHandle, derive_seed,
};
use super::encoding::{
    derivation_transcript, validate_candidate_record, validate_operation_record,
};
use super::{actor_digest, configuration_digest, operation_id};
use crate::seed_binding::canonicalize_seed;

pub(in crate::management) fn candidate_record(
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
        state: SeedBindingStateV2::CandidatePersisted,
    };
    validate_candidate_record(&record)?;
    Ok(record)
}

/// Selects and validates the current key identity without deriving an
/// effective seed. The returned record must be committed by the store before
/// `derive_candidate_from_operation` may be called.
pub(in crate::management) fn prepare_seed_operation(
    request: &WorkflowRunRequestV2,
    actor: &AuthContext,
    request_digest: &str,
    workflow_run_id: &str,
    binding: &TargetAdmissionBinding,
    key_authority: Option<&dyn SeedDerivationKeyAuthority>,
) -> Result<StoredSeedOperationV2, SeedKeyError> {
    if request.seed.mode != SeedModeV2::DeriveOnce {
        return Err(SeedKeyError::InvalidIdentity);
    }
    let authority = key_authority.ok_or(SeedKeyError::Unavailable)?;
    let key = authority.current_key()?;
    key.identity().validate()?;
    if !key.verifies_identity(key.identity())? {
        return Err(SeedKeyError::InvalidKeyMaterial);
    }
    let record = StoredSeedOperationV2 {
        schema_version: WORKFLOW_SEED_OPERATION_V2_SCHEMA.to_owned(),
        request_id: request.request_id.clone(),
        actor_digest: actor_digest(&actor.subject)?,
        request_digest: request_digest.to_owned(),
        workflow_run_id: workflow_run_id.to_owned(),
        operation_id: operation_id(&request.request_id, workflow_run_id, request_digest)?,
        mode: SeedModeV2::DeriveOnce,
        admitted_configuration: binding.clone(),
        configuration_digest: configuration_digest(binding)?,
        derivation: StoredSeedDerivationV2 {
            algorithm_id: SEED_DERIVATION_ALGORITHM_V1.to_owned(),
            key: key.identity().clone(),
        },
        phase: SeedOperationPhaseV2::KeyPinned,
    };
    validate_operation_record(&record)?;
    Ok(record)
}

/// Derives only from a committed operation record and an exact historical
/// key handle loaded by identity. The provisional current key is never an
/// authority once a stored reservation exists.

pub(in crate::management) fn derive_candidate_from_operation(
    request: &WorkflowRunRequestV2,
    actor: &AuthContext,
    request_digest: &str,
    operation: &StoredSeedOperationV2,
    key: &SeedKeyHandle,
) -> Result<StoredSeedBindingV2, SeedKeyError> {
    validate_operation_record(operation)?;
    let expected_operation_id = operation_id(
        &request.request_id,
        &operation.workflow_run_id,
        request_digest,
    )?;
    if request.seed.mode != SeedModeV2::DeriveOnce
        || operation.phase != SeedOperationPhaseV2::KeyPinned
        || operation.request_id != request.request_id
        || operation.request_digest != request_digest
        || operation.actor_digest != actor_digest(&actor.subject)?
        || operation.operation_id != expected_operation_id
        || key.identity() != &operation.derivation.key
        || !key.verifies_identity(&operation.derivation.key)?
    {
        return Err(SeedKeyError::InvalidKeyMaterial);
    }
    let transcript = derivation_transcript(
        key,
        &actor.subject,
        request_digest,
        &operation.workflow_run_id,
        &operation.operation_id,
        &operation.configuration_digest,
    )?;
    let effective_seed = derive_seed(key, &transcript)?;
    let record = StoredSeedBindingV2 {
        schema_version: WORKFLOW_SEED_BINDING_V2_SCHEMA.to_owned(),
        request_id: operation.request_id.clone(),
        actor_digest: operation.actor_digest.clone(),
        request_digest: operation.request_digest.clone(),
        workflow_run_id: operation.workflow_run_id.clone(),
        operation_id: operation.operation_id.clone(),
        mode: SeedModeV2::DeriveOnce,
        requested_seed: None,
        effective_seed,
        derivation: Some(operation.derivation.clone()),
        admitted_configuration: operation.admitted_configuration.clone(),
        configuration_digest: operation.configuration_digest.clone(),
        state: SeedBindingStateV2::CandidatePersisted,
    };
    validate_candidate_record(&record)?;
    Ok(record)
}

pub(in crate::management) fn load_pinned_operation_key(
    operation: &StoredSeedOperationV2,
    key_authority: Option<&dyn SeedDerivationKeyAuthority>,
) -> Result<SeedKeyHandle, SeedKeyError> {
    validate_operation_record(operation)?;
    let authority = key_authority.ok_or(SeedKeyError::Unavailable)?;
    let pinned = &operation.derivation.key;
    let key = authority
        .key_for(&pinned.authority_id, &pinned.version)?
        .ok_or(SeedKeyError::Unavailable)?;
    if key.identity() != pinned || !key.verifies_identity(pinned)? {
        return Err(SeedKeyError::InvalidKeyMaterial);
    }
    Ok(key)
}

pub(in crate::management) fn verify_candidate(
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

pub(in crate::management) fn verify_stored_record(
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
