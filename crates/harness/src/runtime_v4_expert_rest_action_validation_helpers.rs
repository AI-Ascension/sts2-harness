// SPDX-License-Identifier: MIT

fn exact_fields(
    value: &Map<String, Value>,
    fields: &[&str],
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    if value.len() != fields.len() || fields.iter().any(|field| !value.contains_key(*field)) {
        Err(RuntimeV4ExpertRestActionParseError::InvalidShape)
    } else {
        Ok(())
    }
}

fn require_null(value: &Value) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    value
        .is_null()
        .then_some(())
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}

fn number(value: &Value) -> Result<u64, RuntimeV4ExpertRestActionParseError> {
    value
        .as_u64()
        .filter(|number| *number <= MAX_SAFE_INTEGER)
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}

fn bounded_u16(value: &Value) -> Result<u16, RuntimeV4ExpertRestActionParseError> {
    value
        .as_u64()
        .filter(|number| *number <= u16::MAX as u64)
        .and_then(|number| u16::try_from(number).ok())
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}

fn signed_bounded(value: &Value) -> Result<i64, RuntimeV4ExpertRestActionParseError> {
    value
        .as_i64()
        .filter(|number| (-65_535..=65_535).contains(number))
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}

fn positive_count(value: &Value) -> Result<usize, RuntimeV4ExpertRestActionParseError> {
    number(value)
        .ok()
        .filter(|value| (1..=MAX_SELECTOR_ITEMS as u64).contains(value))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}

fn bounded_count(value: &Value) -> Result<usize, RuntimeV4ExpertRestActionParseError> {
    number(value)
        .ok()
        .filter(|value| *value <= MAX_SELECTOR_ITEMS as u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}

fn bounded_unique_ids(value: &Value) -> Result<Vec<&str>, RuntimeV4ExpertRestActionParseError> {
    let ids = value
        .as_array()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    if ids.len() > MAX_SELECTOR_ITEMS {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id
            .as_str()
            .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
        if !identity(&Value::String(id.to_owned())) || !seen.insert(id) {
            return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
        }
        result.push(id);
    }
    Ok(result)
}

fn identity(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        !value.is_empty()
            && value.len() <= MAX_IDENTITY_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            })
    })
}
