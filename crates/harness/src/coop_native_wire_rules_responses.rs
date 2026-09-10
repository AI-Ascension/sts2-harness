// SPDX-License-Identifier: MIT

fn parse_effect_response(
    root: &Map<String, Value>,
) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    require_null(root, "actor_peer")?;
    require_null(root, "expected_host_generation")?;
    require_null(root, "action")?;
    require_null(root, "vote")?;
    require_null(root, "recovery")?;
    require_null(root, "catalog")?;
    let operation_id = required_id(root, "operation_id")?;
    let status = required_status(root, "status")?;
    let observation = parse_observation(
        root.get("observation")
            .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
    )?;
    let effect = optional_value(root, "effect")?.map(parse_effect).transpose()?;
    let receipt = parse_receipt(
        root.get("receipt")
            .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
    )?;
    if (status == CoopNativeStatus::Settled) != effect.is_some()
        || receipt.operation_id != operation_id
        || receipt.status != status
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    if let Some(effect) = &effect
        && (effect.operation_id != operation_id
            || effect.to_generation != observation.host_generation
            || effect.state_digest != observation.state_digest
            || effect.authority_epoch != observation.host_authority_epoch
            || effect.checkpoint_id != observation.checkpoint_id
            || effect.authority_id != observation.authority_id
            || receipt.before_host_generation != effect.from_generation
            || receipt.after_host_generation != Some(effect.to_generation))
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    if receipt.state_digest != observation.state_digest
        || receipt.authority_id != observation.authority_id
        || receipt.authority_epoch != observation.host_authority_epoch
        || receipt.checkpoint_id != observation.checkpoint_id
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(CoopNativeBody::EffectResponse(CoopNativeEffectResponse {
        operation_id,
        status,
        observation,
        effect,
        receipt,
    }))
}
fn parse_recovery_response(
    root: &Map<String, Value>,
) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    require_null(root, "actor_peer")?;
    require_null(root, "expected_host_generation")?;
    require_null(root, "action")?;
    require_null(root, "vote")?;
    require_null(root, "effect")?;
    require_null(root, "catalog")?;
    let operation_id = required_id(root, "operation_id")?;
    let recovery = parse_recovery(
        root.get("recovery")
            .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
    )?;
    let status = optional_status(root, "status")?;
    let observation = optional_value(root, "observation")?
        .map(parse_observation)
        .transpose()?;
    let receipt = optional_value(root, "receipt")?
        .map(parse_receipt)
        .transpose()?;
    match status {
        None if observation.is_some()
            || receipt.is_some()
            || recovery.kind != CoopNativeRecoveryKind::Reconcile =>
        {
            return Err(CoopNativeEnvelopeError::InvalidValue);
        }
        None => {}
        Some(CoopNativeStatus::Accepted) => return Err(CoopNativeEnvelopeError::InvalidValue),
        Some(CoopNativeStatus::Settled | CoopNativeStatus::Rejected) => {
            if observation.is_none()
                || receipt.is_none()
                || recovery.kind != CoopNativeRecoveryKind::Reconcile
            {
                return Err(CoopNativeEnvelopeError::InvalidValue);
            }
        }
        Some(CoopNativeStatus::Unknown)
            if observation.is_none() || receipt.is_none() =>
        {
            return Err(CoopNativeEnvelopeError::InvalidValue);
        }
        Some(CoopNativeStatus::Unknown) => {}
    }
    if let (Some(observation), Some(receipt)) = (&observation, &receipt)
        && (receipt.operation_id != operation_id
            || receipt.state_digest != observation.state_digest
            || receipt.authority_id != observation.authority_id
            || receipt.authority_epoch != observation.host_authority_epoch
            || receipt.checkpoint_id != observation.checkpoint_id
            || !matches!(
                (status, receipt.status),
                (Some(CoopNativeStatus::Unknown), CoopNativeStatus::Accepted | CoopNativeStatus::Unknown)
                    | (Some(CoopNativeStatus::Settled), CoopNativeStatus::Settled)
                    | (Some(CoopNativeStatus::Rejected), CoopNativeStatus::Rejected)
            ))
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(CoopNativeBody::RecoveryResponse(CoopNativeRecoveryResponse {
        operation_id,
        status,
        observation,
        recovery,
        receipt,
    }))
}
