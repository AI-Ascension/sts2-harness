// SPDX-License-Identifier: MIT

use std::path::{Component, Path};

const WINDOWS_NAMESPACE: &str = r"\\.\pipe\ascension-worker-";
const MAX_PATH_BYTES: usize = 4 * 1024;

fn validate_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(String::from("worker bootstrap component is invalid"));
    }
    Ok(())
}

fn parse_creation_token(value: &str) -> Result<u64, String> {
    if value.is_empty()
        || value.len() > 20
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(String::from("worker bootstrap creation token is invalid"));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| String::from("worker bootstrap creation token is invalid"))
}

fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(String::from("worker bootstrap digest is invalid"));
    }
    Ok(())
}

fn validate_sid(value: &str) -> Result<(), String> {
    let mut parts = value.split('-');
    if parts.next() != Some("S") || parts.next() != Some("1") {
        return Err(String::from("worker bootstrap SID is invalid"));
    }
    let authority = parts
        .next()
        .filter(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| String::from("worker bootstrap SID is invalid"))?;
    if authority
        .parse::<u64>()
        .ok()
        .filter(|value| *value < (1_u64 << 48))
        .is_none()
    {
        return Err(String::from("worker bootstrap SID is invalid"));
    }
    let subauthorities = parts.collect::<Vec<_>>();
    if subauthorities.is_empty()
        || subauthorities.len() > 15
        || subauthorities.iter().any(|part| {
            part.is_empty() || part.len() > 10 || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(String::from("worker bootstrap SID is invalid"));
    }
    Ok(())
}

fn validate_windows_path(path: &Path, label: &str) -> Result<(), String> {
    let value = path
        .to_str()
        .ok_or_else(|| format!("{label} path is not Unicode"))?;
    let bytes = value.as_bytes();
    if value.len() < 3
        || value.len() > MAX_PATH_BYTES
        || !path.is_absolute()
        || bytes[1] != b':'
        || bytes[2] != b'\\'
        || value.contains('\0')
        || bytes[2..].contains(&b':')
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(format!("{label} path is invalid"));
    }
    Ok(())
}

fn validate_reference(path: &Path, label: &str) -> Result<(), String> {
    validate_windows_path(path, label)?;
    if path.file_name().is_none() {
        return Err(format!("{label} path is invalid"));
    }
    Ok(())
}

fn validate_namespace(path: &Path) -> Result<(), String> {
    if path.to_str() != Some(WINDOWS_NAMESPACE) {
        return Err(String::from("worker endpoint namespace is invalid"));
    }
    Ok(())
}

fn derive_endpoint(namespace: &Path, nonce: &str) -> Result<String, String> {
    validate_namespace(namespace)?;
    let uuid = uuid::Uuid::parse_str(nonce)
        .map_err(|_| String::from("worker endpoint nonce is invalid"))?;
    if uuid.get_version_num() != 4 || uuid.get_variant() != uuid::Variant::RFC4122 {
        return Err(String::from("worker endpoint nonce is invalid"));
    }
    Ok(format!("{WINDOWS_NAMESPACE}{nonce}"))
}

fn fresh_worker_boot_id(watchdog_boot_id: &str) -> String {
    loop {
        let value = uuid::Uuid::new_v4().to_string();
        if value != watchdog_boot_id {
            return value;
        }
    }
}

fn required_env(name: &str) -> Result<String, String> {
    optional_env(name)?.ok_or_else(|| format!("{name} is required"))
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not UTF-8")),
    }
}

fn alias_env(primary: &str, alias: &str) -> Result<Option<String>, String> {
    let primary_value = optional_env(primary)?;
    let alias_value = optional_env(alias)?;
    if primary_value.is_some() && alias_value.is_some() && primary_value != alias_value {
        return Err(format!("{primary} and {alias} disagree"));
    }
    Ok(primary_value.or(alias_value))
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    required_env(name).map(PathBuf::from)
}

fn optional_path(name: &str) -> Result<Option<PathBuf>, String> {
    optional_env(name).map(|value| value.map(PathBuf::from))
}

fn current_runtime_binary() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|_| String::from("worker runtime executable is unavailable"))
}

fn approved_environment() -> Vec<(OsString, OsString)> {
    const DENY: &[&str] = &[
        "STS2_WORKER_ENDPOINT_NAMESPACE",
        "STS2_WORKER_CREDENTIAL_PATH",
        "STS2_WORKER_RUNTIME_BINARY",
        "STS2_WORKER_RUNTIME_SHA256",
        "STS2_WORKER_TIMEOUT_MS",
    ];
    std::env::vars_os()
        .filter(|(key, _)| {
            let text = key.to_string_lossy();
            (text.starts_with("STS2_")
                || matches!(
                    text.as_ref(),
                    "PATH" | "USERPROFILE" | "TEMP" | "TMP" | "SystemRoot"
                ))
                && !DENY.iter().any(|denied| *denied == text)
                && text != "STS2_ATTEMPT_ID"
        })
        .collect()
}
