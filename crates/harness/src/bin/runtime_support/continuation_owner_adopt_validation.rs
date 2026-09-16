// SPDX-License-Identifier: MIT

fn validate_owner(value: &Value) -> Result<(), String> {
    let _ = exact_object(
        value,
        &[
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "host_fence_id",
            "host_fence_generation",
            "lease_id",
            "lease_epoch",
            "session_id",
            "lease_expires_at_millis",
        ],
        "owner",
    )?;
    for field in ["deployment_id", "instance_id"] {
        if !uuid(value[field].as_str(), false) {
            return Err(String::from(
                "gateway adopted owner contains an invalid identity",
            ));
        }
    }
    for field in [
        "instance_incarnation",
        "boot_id",
        "host_fence_id",
        "lease_id",
    ] {
        if !uuid(value[field].as_str(), true) {
            return Err(String::from(
                "gateway adopted owner contains an invalid identity",
            ));
        }
    }
    if value["session_id"]
        .as_str()
        .is_none_or(|session| session.is_empty() || session.len() > 512)
    {
        return Err(String::from(
            "gateway adopted owner contains an invalid session identity",
        ));
    }
    for field in [
        "authority_generation",
        "host_fence_generation",
        "lease_epoch",
        "lease_expires_at_millis",
    ] {
        if positive_u53(&value[field]).is_none() {
            return Err(String::from(
                "gateway adopted owner contains an invalid fence counter",
            ));
        }
    }
    Ok(())
}

fn exact_object<'a>(
    value: &'a Value,
    fields: &[&str],
    label: &str,
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("gateway continuation-owner {label} is not an object"))?;
    if object.len() != fields.len()
        || object.keys().any(|key| !fields.contains(&key.as_str()))
        || fields.iter().any(|field| !object.contains_key(*field))
    {
        return Err(format!(
            "gateway continuation-owner {label} has unexpected fields"
        ));
    }
    Ok(object)
}

fn uuid(value: Option<&str>, v4: bool) -> bool {
    value.is_some_and(|value| {
        Uuid::parse_str(value).ok().is_some_and(|uuid| {
            uuid.hyphenated().to_string() == value
                && uuid.get_variant() == uuid::Variant::RFC4122
                && match uuid.get_version_num() {
                    1..=8 if !v4 => true,
                    4 if v4 => true,
                    _ => false,
                }
        })
    })
}

fn uuid_v4(value: Option<&str>) -> bool {
    uuid(value, true)
}

fn lower_sha256(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn positive_u53(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .filter(|number| *number > 0 && *number <= MAX_SAFE_INTEGER)
}

fn now_millis() -> Result<u64, String> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| String::from("system clock is before the Unix epoch"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| String::from("system clock is out of range"))
}
