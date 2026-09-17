// SPDX-License-Identifier: MIT

const COMPATIBILITY_ENV: &str = "STS2_EXACT_RESTORE_COMPATIBILITY_DIGEST";
const COVERAGE_ENV: &str = "STS2_EXACT_RESTORE_COVERAGE_CONTRACT_DIGEST";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProfilePins {
    pub(super) compatibility_digest: String,
    pub(super) coverage_contract_digest: String,
}

impl ProfilePins {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let compatibility_digest = required_pin(COMPATIBILITY_ENV)?;
        let coverage_contract_digest = required_pin(COVERAGE_ENV)?;
        Ok(Self {
            compatibility_digest,
            coverage_contract_digest,
        })
    }
}

fn required_pin(name: &str) -> Result<String, String> {
    let value = optional_environment_value(name)?
        .ok_or_else(|| format!("{name} is required for exact branch continuation"))?;
    validate_pin(name, &value)?;
    Ok(value)
}

fn optional_environment_value(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub(crate) fn validate_pin(name: &str, value: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(format!("{name} must use sha256:<64 lowercase hex>"));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{name} must use sha256:<64 lowercase hex>"));
    }
    Ok(())
}
