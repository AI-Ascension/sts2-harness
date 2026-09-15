// SPDX-License-Identifier: MIT

use std::sync::OnceLock;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::contract::{Availability, Document, InferenceSeed, Occurrence, SelectedContext};
use super::{MAX_MANIFEST_BYTES, ManifestError, VERSION};

pub(super) fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ManifestError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge);
    }
    let value = crate::recorded_run::recorded_run_json::parse(bytes)
        .map_err(|_| ManifestError::InvalidDocument)?;
    serde_json::from_value(value).map_err(|_| ManifestError::InvalidDocument)
}

pub(super) fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(&b))
}

pub(super) fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn seed(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
}

fn known(value: &Availability) -> bool {
    match value {
        Availability::Known { value } => token(value),
        Availability::Unavailable => true,
    }
}

pub(super) fn document(doc: &Document) -> Result<(), ManifestError> {
    if doc.version != VERSION {
        return Err(ManifestError::UnsupportedVersion);
    }
    let g = &doc.gameplay;
    let c = &g.selected_context;
    let e = &doc.experiment;
    context(c)?;
    if !seed(&g.requested_seed) || !seed(&g.effective_seed) || !token(&g.seed_contract) {
        return Err(ManifestError::InvalidSeed);
    }
    if c.profile_baseline.kind != "fresh"
        || !opaque_reference(&g.profile_artifact.reference)
        || g.profile_artifact.baseline_digest != c.profile_baseline.digest
    {
        return Err(ManifestError::InvalidProfile);
    }
    if g.assembly_hashes.is_empty()
        || g.assembly_hashes.len() > 32
        || g.assembly_hashes
            .iter()
            .any(|(k, v)| !token(k) || !digest(v))
    {
        return Err(ManifestError::InvalidCompatibility);
    }
    compatibility(doc)?;
    if [&e.provider, &e.model, &e.evaluator_revision]
        .iter()
        .any(|v| !token(v))
        || !known(&e.provider_revision)
        || !known(&e.model_revision)
        || e.inference_parameters.len() > 32
        || e.inference_parameters.iter().any(|(k, v)| {
            !token(k) || v.is_empty() || v.len() > 128 || v.chars().any(char::is_control)
        })
        || matches!(&e.inference_seed, InferenceSeed::Requested { value, .. } if !seed(value))
    {
        return Err(ManifestError::InvalidExperiment);
    }
    if !(1..=1_000_000).contains(&e.budgets.max_decisions)
        || !(1..=1_000_000_000).contains(&e.budgets.max_tokens)
        || !(1..=604_800_000).contains(&e.budgets.max_duration_ms)
    {
        return Err(ManifestError::InvalidBudget);
    }
    Ok(())
}

fn compatibility(doc: &Document) -> Result<(), ManifestError> {
    let g = &doc.gameplay;
    let e = &doc.experiment;
    let p = &g.platform;
    if [
        &g.protocol_version,
        &g.game_version,
        &g.coverage_version,
        &p.os,
        &p.architecture,
        &p.runtime_version,
        &p.compatibility_contract,
    ]
    .iter()
    .any(|v| !token(v))
        || [
            &g.protocol_digest,
            &g.coverage_digest,
            &g.unlock_progress_digest,
            &g.gameplay_settings_digest,
            &e.prompt_digest,
            &e.workflow_digest,
            &e.context_digest,
            &e.tool_policy_digest,
        ]
        .iter()
        .any(|v| !digest(v))
    {
        return Err(ManifestError::InvalidCompatibility);
    }
    for component in [
        &g.components.harness,
        &g.components.mcp,
        &g.components.gateway,
        &g.components.game_mod,
    ] {
        if component.revision.len() != 40
            || !component
                .revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !digest(&component.package_digest)
        {
            return Err(ManifestError::InvalidCompatibility);
        }
    }
    Ok(())
}

fn opaque_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

// Use the immutable protocol artifact as structural authority, including bounds,
// enums and unique act IDs. No schema/network resolution is required.
pub(super) fn schema_valid(value: &Value, context_only: bool) -> Result<(), ManifestError> {
    static CONTEXT: OnceLock<Result<jsonschema::Validator, ()>> = OnceLock::new();
    static RECEIPT: OnceLock<Result<jsonschema::Validator, ()>> = OnceLock::new();
    let cache = if context_only { &CONTEXT } else { &RECEIPT };
    let validator = cache
        .get_or_init(|| {
            let mut schema: Value = serde_json::from_str(include_str!(
                "../../../../protocol-artifact/seeded-run-v1/schema.json"
            ))
            .map_err(|_| ())?;
            if context_only {
                schema.as_object_mut().ok_or(())?.remove("oneOf");
                schema["$ref"] = Value::String("#/$defs/selected_context".to_owned());
            }
            jsonschema::validator_for(&schema).map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| ManifestError::InvalidDocument)?;
    if validator.is_valid(value) {
        Ok(())
    } else {
        Err(ManifestError::InvalidDocument)
    }
}

fn context(context: &SelectedContext) -> Result<(), ManifestError> {
    let value = serde_json::to_value(context).map_err(|_| ManifestError::InvalidDocument)?;
    schema_valid(&value, true)?;
    // The field is last in the accepted typed representation. Removing this exact
    // serialized suffix preserves ContextDigestInput's field order, unlike Value.
    let encoded = serde_json::to_string(context).map_err(|_| ManifestError::InvalidDocument)?;
    let suffix = format!(",\"context_digest\":\"{}\"}}", context.context_digest);
    let prefix = encoded
        .strip_suffix(&suffix)
        .ok_or(ManifestError::InvalidDocument)?;
    let input = format!("{prefix}}}");
    if crate::sha256_hex(input.as_bytes()) != context.context_digest
        || context.modifiers.windows(2).any(|p| p[0] >= p[1])
    {
        return Err(ManifestError::InvalidContextDigest);
    }
    Ok(())
}

pub(super) fn occurrence(o: &Occurrence) -> Result<(), ManifestError> {
    if [
        &o.process_id,
        &o.run_id,
        &o.episode_id,
        &o.instance_id,
        &o.gateway_session_id,
        &o.mcp_session_id,
        &o.lease_id,
        &o.operation_id,
    ]
    .iter()
    .any(|v| !token(v))
        || !digest(&o.plan_digest)
        || o.entry_ordinal > 1023
        || o.lease_epoch > 9_007_199_254_740_991
        || o.request_generation > 9_007_199_254_740_991
        || o.created_at_unix_ms > 9_007_199_254_740_991
        || !matches!(
            o.run_mode.as_str(),
            "seeded_training" | "seeded_replay" | "diagnostic"
        )
    {
        return Err(ManifestError::InvalidOccurrence);
    }
    Ok(())
}
