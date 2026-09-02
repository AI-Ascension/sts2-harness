// SPDX-License-Identifier: MIT

fn stored_binding(stored: &StoredOperation) -> &OperationBinding {
    match stored {
        StoredOperation::Admission { binding, .. } | StoredOperation::Terminal { binding, .. } => {
            binding
        }
    }
}

fn replay_stored(
    stored: &StoredOperation,
    request: &RuntimeV2Message,
) -> Result<EngineResult, RuntimeV2Error> {
    let binding = OperationBinding::from_message(request)?;
    if stored_binding(stored) != &binding {
        return Err(RuntimeV2Error::Invalid(
            "stored operation context does not match replay request",
        ));
    }
    let mut message = match stored {
        StoredOperation::Admission {
            accepted, settled, ..
        } => settled.clone().unwrap_or_else(|| accepted.clone()),
        StoredOperation::Terminal { result, .. } => result.clone(),
    };
    if message.operation_id() != request.operation_id() {
        return Err(RuntimeV2Error::Invalid(
            "stored operation identity does not match replay request",
        ));
    }
    message.correlation_id = request.correlation_id().to_owned();
    message.validate()?;
    Ok(EngineResult {
        message,
        replayed: true,
    })
}

fn no_retry_evidence(
    operation_id: Option<RuntimeV2OperationId>,
    disconnect_after_write: bool,
    mutation_attempts: u8,
) -> RuntimeV2NoRetryEvidence {
    RuntimeV2NoRetryEvidence::new(
        true,
        0,
        mutation_attempts,
        disconnect_after_write,
        operation_id,
    )
}

fn push_record(
    records: &mut Vec<RuntimeV2Record>,
    event_kind: RuntimeV2EventKind,
    record_kind: RuntimeV2RecordKind,
    message: RuntimeV2Message,
    no_retry: RuntimeV2NoRetryEvidence,
) -> Result<(), RuntimeV2Error> {
    let sequence = u16::try_from(records.len())
        .map_err(|_| RuntimeV2Error::Invalid("Runtime-v2 record sequence overflowed"))?;
    records.push(RuntimeV2Record::new(
        sequence,
        event_kind,
        record_kind,
        message,
        no_retry,
    )?);
    Ok(())
}

fn validate_identity(value: &str) -> Result<(), RuntimeV2Error> {
    if value.is_empty()
        || value.len() > RUNTIME_V2_MAX_IDENTITY_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-/".contains(&byte))
    {
        return Err(RuntimeV2Error::Invalid(
            "Runtime-v2 identity is empty, unsafe, or too long",
        ));
    }
    Ok(())
}
