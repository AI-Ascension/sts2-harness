// SPDX-License-Identifier: MIT

fn exact_fields(
    object: &Map<String, Value>,
    fields: &[&str],
) -> Result<(), CoopNativeEnvelopeError> {
    if object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field)) {
        Ok(())
    } else {
        Err(CoopNativeEnvelopeError::InvalidShape)
    }
}
fn text<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, CoopNativeEnvelopeError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(CoopNativeEnvelopeError::InvalidValue)
}

fn required_text_identity(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, CoopNativeEnvelopeError> {
    let value = text(object, field)?;
    super::identity::validate_identity(value).map_err(CoopNativeEnvelopeError::from)?;
    Ok(value.to_owned())
}

fn required_id<T: NativeId>(
    object: &Map<String, Value>,
    field: &str,
) -> Result<T, CoopNativeEnvelopeError> {
    T::parse(text(object, field)?.to_owned()).map_err(Into::into)
}

fn optional_id<T: NativeId>(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<T>, CoopNativeEnvelopeError> {
    let value = object
        .get(field)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    if value.is_null() {
        Ok(None)
    } else {
        T::parse(text(object, field)?.to_owned())
            .map(Some)
            .map_err(Into::into)
    }
}

fn optional_identity(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<String>, CoopNativeEnvelopeError> {
    let value = object
        .get(field)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    if value.is_null() {
        Ok(None)
    } else {
        required_text_identity(object, field).map(Some)
    }
}

fn required_peer(
    object: &Map<String, Value>,
    field: &str,
) -> Result<CoopNativePeerId, CoopNativeEnvelopeError> {
    required_id(object, field)
}

fn optional_peer(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<CoopNativePeerId>, CoopNativeEnvelopeError> {
    optional_id(object, field)
}

fn generation(
    object: &Map<String, Value>,
    field: &str,
) -> Result<u64, CoopNativeEnvelopeError> {
    let value = object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(CoopNativeEnvelopeError::InvalidValue)?;
    validate_generation(value).map_err(CoopNativeEnvelopeError::from)?;
    Ok(value)
}

fn optional_generation(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<u64>, CoopNativeEnvelopeError> {
    let value = object
        .get(field)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    if value.is_null() {
        Ok(None)
    } else {
        generation(object, field).map(Some)
    }
}

fn boolean(object: &Map<String, Value>, field: &str) -> Result<bool, CoopNativeEnvelopeError> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or(CoopNativeEnvelopeError::InvalidValue)
}

fn digest(value: &Value) -> Result<String, CoopNativeEnvelopeError> {
    let value = value
        .as_str()
        .ok_or(CoopNativeEnvelopeError::InvalidValue)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(value.to_owned())
}

fn required_digest(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, CoopNativeEnvelopeError> {
    digest(object.get(field).ok_or(CoopNativeEnvelopeError::InvalidShape)?)
}

fn optional_digest(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<String>, CoopNativeEnvelopeError> {
    let value = object
        .get(field)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    if value.is_null() {
        Ok(None)
    } else {
        digest(value).map(Some)
    }
}

fn required_checksum_status(
    object: &Map<String, Value>,
    field: &str,
) -> Result<CoopNativeChecksumStatus, CoopNativeEnvelopeError> {
    CoopNativeChecksumStatus::parse(text(object, field)?)
        .ok_or(CoopNativeEnvelopeError::InvalidValue)
}

fn required_status(
    object: &Map<String, Value>,
    field: &str,
) -> Result<CoopNativeStatus, CoopNativeEnvelopeError> {
    CoopNativeStatus::parse(text(object, field)?).ok_or(CoopNativeEnvelopeError::InvalidValue)
}

fn optional_status(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<CoopNativeStatus>, CoopNativeEnvelopeError> {
    let value = object
        .get(field)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    if value.is_null() {
        Ok(None)
    } else {
        required_status(object, field).map(Some)
    }
}

fn optional_value<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a Value>, CoopNativeEnvelopeError> {
    let value = object
        .get(field)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    Ok((!value.is_null()).then_some(value))
}

fn require_null(object: &Map<String, Value>, field: &str) -> Result<(), CoopNativeEnvelopeError> {
    object
        .get(field)
        .filter(|value| value.is_null())
        .map(|_| ())
        .ok_or(CoopNativeEnvelopeError::InvalidValue)
}
