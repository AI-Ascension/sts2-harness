// SPDX-License-Identifier: MIT

// SPDX-License-Identifier: MIT

fn parse_observation(value: &Value) -> Result<CoopNativeObservation, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(object, &OBSERVATION_FIELDS)?;
    let peers_value = object
        .get("peers")
        .and_then(Value::as_array)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    if !(2..=4).contains(&peers_value.len()) {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    if text(object, "host_sequence_kind")? != "adapter_sequence"
        || text(object, "checksum_algorithm")? != "sha256"
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    let peers = peers_value
        .iter()
        .map(parse_peer)
        .collect::<Result<Vec<_>, _>>()?;
    if peers
        .iter()
        .enumerate()
        .any(|(index, peer)| peers[..index].iter().any(|other| other == peer))
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    if peers
        .iter()
        .filter(|peer| peer.role == CoopNativePeerRole::Local)
        .count()
        != 1
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(CoopNativeObservation {
        host_authority_epoch: required_id(object, "host_authority_epoch")?,
        authority_id: required_id(object, "authority_id")?,
        run_id: required_id(object, "run_id")?,
        host_sequence_kind: "adapter_sequence".to_owned(),
        host_generation: generation(object, "host_generation")?,
        state_digest: required_digest(object, "state_digest")?,
        checkpoint_id: required_id(object, "checkpoint_id")?,
        checksum_algorithm: "sha256".to_owned(),
        checksum_status: required_checksum_status(object, "checksum_status")?,
        native_checksum: optional_digest(object, "native_checksum")?,
        host_digest_known: boolean(object, "host_digest_known")?,
        host_loading: boolean(object, "host_loading")?,
        host_divergent: boolean(object, "host_divergent")?,
        peers,
    })
}
fn parse_peer(value: &Value) -> Result<CoopNativePeerSnapshot, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(object, &PEER_FIELDS)?;
    Ok(CoopNativePeerSnapshot {
        peer_token: required_peer(object, "peer_token")?,
        authority_id: required_id(object, "authority_id")?,
        role: CoopNativePeerRole::parse(text(object, "role")?)
            .ok_or(CoopNativeEnvelopeError::InvalidValue)?,
        connected: boolean(object, "connected")?,
        peer_generation: generation(object, "peer_generation")?,
        state_digest: required_digest(object, "state_digest")?,
        rejoin_epoch: generation(object, "rejoin_epoch")?,
        authority_epoch: required_id(object, "authority_epoch")?,
        checkpoint_id: optional_id(object, "checkpoint_id")?,
        digest_known: boolean(object, "digest_known")?,
        is_loading: boolean(object, "is_loading")?,
        is_divergent: boolean(object, "is_divergent")?,
        checksum_status: required_checksum_status(object, "checksum_status")?,
    })
}

fn parse_action(value: &Value) -> Result<CoopNativeAction, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(object, &ACTION_FIELDS)?;
    Ok(CoopNativeAction {
        kind: CoopNativeActionKind::parse(text(object, "kind")?)
            .ok_or(CoopNativeEnvelopeError::InvalidValue)?,
        action_id: required_id(object, "action_id")?,
        target_peer: optional_peer(object, "target_peer")?,
    })
}

fn parse_vote(value: &Value) -> Result<CoopNativeVote, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(object, &VOTE_FIELDS)?;
    Ok(CoopNativeVote {
        proposal_id: required_id(object, "proposal_id")?,
        voter_peer: required_peer(object, "voter_peer")?,
        choice: required_text_identity(object, "choice")?,
    })
}

fn parse_recovery(value: &Value) -> Result<CoopNativeRecovery, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(object, &RECOVERY_FIELDS)?;
    Ok(CoopNativeRecovery {
        kind: CoopNativeRecoveryKind::parse(text(object, "kind")?)
            .ok_or(CoopNativeEnvelopeError::InvalidValue)?,
        rejoin_epoch: generation(object, "rejoin_epoch")?,
    })
}

fn parse_catalog(value: &Value) -> Result<CoopNativeCatalog, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(object, &CATALOG_FIELDS)?;
    let actor_peer = required_peer(object, "actor_peer")?;
    let actions = object
        .get("actions")
        .and_then(Value::as_array)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?
        .iter()
        .map(parse_action)
        .collect::<Result<Vec<_>, _>>()?;
    let votes = object
        .get("votes")
        .and_then(Value::as_array)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?
        .iter()
        .map(parse_vote)
        .collect::<Result<Vec<_>, _>>()?;
    if actions.len() > 256 || votes.len() > 256 {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    let mut ids = std::collections::BTreeSet::new();
    for action in &actions {
        if !ids.insert(action.action_id.as_str().to_owned()) {
            return Err(CoopNativeEnvelopeError::InvalidValue);
        }
    }
    for vote in &votes {
        if vote.voter_peer != actor_peer
            || !ids.insert(format!("vote:{}:{}", vote.proposal_id.as_str(), vote.choice))
        {
            return Err(CoopNativeEnvelopeError::InvalidValue);
        }
    }
    Ok(CoopNativeCatalog {
        host_generation: generation(object, "host_generation")?,
        actor_peer,
        actions,
        votes,
    })
}

fn parse_effect(value: &Value) -> Result<CoopNativeEffect, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    const EFFECT_FIELDS: [&str; 10] = [
        "effect_id",
        "operation_id",
        "kind",
        "from_generation",
        "to_generation",
        "state_digest",
        "authority_epoch",
        "checkpoint_id",
        "authority_id",
        "native_checksum",
    ];
    exact_fields(object, &EFFECT_FIELDS)?;
    let from_generation = generation(object, "from_generation")?;
    let to_generation = generation(object, "to_generation")?;
    if from_generation >= to_generation {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(CoopNativeEffect {
        effect_id: required_id(object, "effect_id")?,
        operation_id: required_id(object, "operation_id")?,
        kind: CoopNativeEffectKind::parse(text(object, "kind")?)
            .ok_or(CoopNativeEnvelopeError::InvalidValue)?,
        from_generation,
        to_generation,
        state_digest: required_digest(object, "state_digest")?,
        authority_epoch: required_id(object, "authority_epoch")?,
        checkpoint_id: required_id(object, "checkpoint_id")?,
        authority_id: required_id(object, "authority_id")?,
        native_checksum: optional_digest(object, "native_checksum")?,
    })
}

fn parse_receipt(value: &Value) -> Result<CoopNativeReceipt, CoopNativeEnvelopeError> {
    let object = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(object, &RECEIPT_FIELDS)?;
    Ok(CoopNativeReceipt {
        operation_id: required_id(object, "operation_id")?,
        status: required_status(object, "status")?,
        before_host_generation: generation(object, "before_host_generation")?,
        after_host_generation: optional_generation(object, "after_host_generation")?,
        authority_id: required_id(object, "authority_id")?,
        authority_epoch: required_id(object, "authority_epoch")?,
        checkpoint_id: required_id(object, "checkpoint_id")?,
        state_digest: required_digest(object, "state_digest")?,
        native_checksum: optional_digest(object, "native_checksum")?,
        error_code: optional_identity(object, "error_code")?,
    })
}
