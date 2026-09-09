// SPDX-License-Identifier: MIT

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    ContextDigestInput, MAX_ACTS, MAX_CONTEXT_TEXT_BYTES, MAX_ENTRIES, MAX_IDENTITY_BYTES,
    MAX_MODIFIERS, MAX_SEED_BYTES, PlanDocument, SelectedContext,
};

pub(super) fn validate_plan(plan: &PlanDocument) -> Result<(), String> {
    if plan.entries.is_empty() || plan.entries.len() > MAX_ENTRIES {
        return Err(String::from(
            "STS2_SEED_PLAN_JSON entries are outside their bound",
        ));
    }
    for (index, entry) in plan.entries.iter().enumerate() {
        if entry.ordinal != index as u64 {
            return Err(String::from(
                "STS2_SEED_PLAN_JSON ordinals must be contiguous",
            ));
        }
        validate_seed(&entry.requested_seed)?;
    }
    Ok(())
}

pub(super) fn validate_context(context: &SelectedContext) -> Result<(), String> {
    identity(&context.context_id, "context_id")?;
    if context.game_mode != "standard" || context.character != "ironclad" || context.ascension > 20
    {
        return Err(String::from(
            "seed context must be standard Ironclad ascension 0..20",
        ));
    }
    if context.acts.is_empty()
        || context.acts.len() > MAX_ACTS
        || context.modifiers.len() > MAX_MODIFIERS
    {
        return Err(String::from(
            "seed context acts or modifiers are outside their bound",
        ));
    }
    for value in &context.acts {
        identity(value, "act")?;
    }
    for value in &context.modifiers {
        identity(value, "modifier")?;
    }
    if context.modifiers.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(String::from(
            "seed context modifiers must be sorted and unique",
        ));
    }
    identity(&context.selection_policy, "selection_policy")?;
    if !matches!(context.profile_baseline.kind.as_str(), "fresh" | "existing") {
        return Err(String::from(
            "seed context profile_baseline.kind is invalid",
        ));
    }
    identity(
        &context.profile_baseline.identity,
        "profile_baseline.identity",
    )?;
    digest(&context.profile_baseline.digest, "profile_baseline.digest")?;
    if !matches!(context.save_policy.as_str(), "disabled" | "enabled") {
        return Err(String::from("seed context save_policy is invalid"));
    }
    for (name, value) in [
        (
            "compatibility.game.identity",
            &context.compatibility.game.identity,
        ),
        (
            "compatibility.mod.identity",
            &context.compatibility.mod_identity.identity,
        ),
    ] {
        identity(value, name)?;
    }
    for (name, value) in [
        (
            "compatibility.game.digest",
            &context.compatibility.game.digest,
        ),
        (
            "compatibility.mod.digest",
            &context.compatibility.mod_identity.digest,
        ),
    ] {
        digest(value, name)?;
    }
    digest(&context.context_digest, "context_digest")
}

fn validate_seed(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_SEED_BYTES || value.chars().any(char::is_control) {
        return Err(String::from(
            "requested seed is empty, unsafe, or oversized",
        ));
    }
    Ok(())
}

fn identity(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_CONTEXT_TEXT_BYTES
        || value.chars().any(char::is_control)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(format!("{name} is empty, unsafe, or oversized"));
    }
    Ok(())
}

pub(super) fn required_identity(name: &str) -> Result<String, String> {
    let value = required(name)?;
    if value.len() > MAX_IDENTITY_BYTES
        || value.chars().any(char::is_control)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(format!("{name} is empty, unsafe, or oversized"));
    }
    Ok(value)
}

fn digest(value: &str, name: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{name} must be a lowercase SHA-256 digest"));
    }
    Ok(())
}

pub(super) fn context_digest(context: &SelectedContext) -> Result<String, String> {
    let input = ContextDigestInput {
        context_id: &context.context_id,
        game_mode: &context.game_mode,
        character: &context.character,
        ascension: context.ascension,
        modifiers: &context.modifiers,
        acts: &context.acts,
        selection_policy: &context.selection_policy,
        profile_baseline: &context.profile_baseline,
        save_policy: &context.save_policy,
        compatibility: &context.compatibility,
    };
    sha256_bytes(&serde_json::to_vec(&input).map_err(|error| error.to_string())?)
}

pub(super) fn sha256_json<T: Serialize>(value: &T) -> Result<String, String> {
    sha256_bytes(&serde_json::to_vec(value).map_err(|error| error.to_string())?)
}

fn sha256_bytes(bytes: &[u8]) -> Result<String, String> {
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

pub(super) fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub(super) fn optional_u64(name: &str) -> Result<Option<u64>, String> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("{name} must be an integer")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}
