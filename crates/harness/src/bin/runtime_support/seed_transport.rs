// SPDX-License-Identifier: MIT

//! Explicit seeded-run launch configuration passed from the harness controller.
//!
//! This module only parses and validates the bounded launch context. It does not
//! claim that a game host accepted the seed; that evidence belongs to the
//! seeded-run MCP receipt and the authoritative host observation.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[path = "seed_transport_reservation.rs"]
mod reservation;
#[cfg(test)]
#[path = "seed_transport_tests.rs"]
mod tests;
#[path = "seed_transport_validation.rs"]
mod validation;

use validation::{
    context_digest, env_or_default, optional_u64, required, required_identity, sha256_json,
    validate_context, validate_plan,
};

const MAX_PLAN_BYTES: usize = 256 * 1024;
const MAX_ENTRIES: usize = 1024;
const MAX_SEED_BYTES: usize = 64;
const MAX_IDENTITY_BYTES: usize = 128;
const MAX_CONTEXT_TEXT_BYTES: usize = 128;
const MAX_MODIFIERS: usize = 32;
const MAX_ACTS: usize = 8;

pub(crate) const SEEDED_RUN_PROTOCOL_VERSION: &str = "seeded-run-v1";
pub(crate) const SEEDED_RUN_SCHEMA_DIGEST: &str =
    "5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8";
pub(crate) const SEEDED_RUN_ARTIFACT: &str = "sts2-protocol/seeded-run-v1";
pub(crate) const SEEDED_RUN_SCHEMA_SOURCE: &str = "schemas/seeded-run-v1.schema.json";
pub(crate) const SEEDED_RUN_GENERATOR: &str = "hand-authored";
pub(crate) const SEEDED_RUN_MAX_GENERATION: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PlanEntry {
    ordinal: u64,
    #[serde(alias = "seed")]
    requested_seed: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PlanDocument {
    entries: Vec<PlanEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct IdentityDigest {
    identity: String,
    digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ProfileBaseline {
    kind: String,
    identity: String,
    digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Compatibility {
    game: IdentityDigest,
    #[serde(rename = "mod")]
    mod_identity: IdentityDigest,
}

/// Concrete native context selected for this one operation. Field order is
/// intentionally fixed because it is the canonical digest input.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct SelectedContext {
    context_id: String,
    game_mode: String,
    character: String,
    ascension: u8,
    modifiers: Vec<String>,
    acts: Vec<String>,
    selection_policy: String,
    profile_baseline: ProfileBaseline,
    save_policy: String,
    compatibility: Compatibility,
    context_digest: String,
}

#[derive(Serialize)]
struct ContextDigestInput<'a> {
    context_id: &'a str,
    game_mode: &'a str,
    character: &'a str,
    ascension: u8,
    modifiers: &'a [String],
    acts: &'a [String],
    selection_policy: &'a str,
    profile_baseline: &'a ProfileBaseline,
    save_policy: &'a str,
    compatibility: &'a Compatibility,
}

/// Validated input carried by the runtime controller into one seeded-run start.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct SeedTransportConfig {
    pub(super) plan_digest: String,
    pub(super) entry_ordinal: u64,
    pub(super) requested_seed: String,
    pub(super) operation_id: String,
    pub(super) run_mode: String,
    pub(super) verify_idempotency: bool,
    context: SelectedContext,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReservationRecord {
    protocol: String,
    operation_id: String,
    plan_digest: String,
    entry_ordinal: u64,
    requested_seed: String,
    run_mode: String,
    context_digest: String,
    selected_context: Value,
    generation: u64,
    status: String,
    settled_receipt: Option<Value>,
}

/// Atomic reservation marker. An existing marker always enters reconcile-only
/// recovery; it is never treated as permission to issue another start.
#[derive(Clone, Debug)]
pub(crate) struct SeedReservation {
    path: PathBuf,
    generation: u64,
    pub(crate) resumed: bool,
}

#[allow(dead_code)]
impl SeedTransportConfig {
    #[cfg(test)]
    #[allow(clippy::panic)]
    pub(super) fn fixture_for_tests(requested_seed: &str, operation_id: &str) -> Self {
        let context: SelectedContext = serde_json::from_str(
            r#"{"context_id":"standard/ironclad/asc0/fresh","game_mode":"standard","character":"ironclad","ascension":0,"modifiers":[],"acts":["act_1","act_2","act_3","act_4"],"selection_policy":"standard_default","profile_baseline":{"kind":"fresh","identity":"fresh-standard-comparison","digest":"4581aaf95348126550cdf3b73ec46b39d447523cf7cb35aec71c2842d1945031"},"save_policy":"disabled","compatibility":{"game":{"identity":"sts2-game/v0.107.1","digest":"2db9d9f665c776c2324c7f98134b900a8b3332f32148ea1cb84063d52db94ff4"},"mod":{"identity":"ai-ascension/sts2-game-mod","digest":"0c6e7bbb54996222a4894de999fc860decc360f9936c9bd5a7382d97b361bb7e"}},"context_digest":"d57563180f198b73970510427981504a9df10c62931577e601f9dcce6275fbe9"}"#,
        )
        .unwrap_or_else(|error| panic!("test context must parse: {error}"));
        Self {
            plan_digest: "a".repeat(64),
            entry_ordinal: 0,
            requested_seed: requested_seed.to_owned(),
            operation_id: operation_id.to_owned(),
            run_mode: String::from("seeded_training"),
            verify_idempotency: true,
            context,
        }
    }

    pub(crate) fn from_environment() -> Result<Option<Self>, String> {
        let raw_plan = match std::env::var("STS2_SEED_PLAN_JSON") {
            Ok(value) if !value.is_empty() => value,
            Ok(_) => return Err(String::from("STS2_SEED_PLAN_JSON must not be empty")),
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(String::from("STS2_SEED_PLAN_JSON is not valid UTF-8"));
            }
        };
        if raw_plan.len() > MAX_PLAN_BYTES {
            return Err(String::from("STS2_SEED_PLAN_JSON exceeds its size limit"));
        }
        let plan: PlanDocument = serde_json::from_str(&raw_plan)
            .map_err(|error| format!("STS2_SEED_PLAN_JSON is invalid: {error}"))?;
        validate_plan(&plan)?;
        let plan_digest = sha256_json(&plan)?;
        if let Ok(expected) = std::env::var("STS2_SEED_PLAN_DIGEST")
            && expected != plan_digest
        {
            return Err(String::from(
                "STS2_SEED_PLAN_DIGEST does not match the plan",
            ));
        }
        let requested_ordinal = optional_u64("STS2_SEED_ENTRY_ORDINAL")?;
        let entry = match requested_ordinal {
            Some(ordinal) => plan
                .entries
                .iter()
                .find(|entry| entry.ordinal == ordinal)
                .ok_or_else(|| String::from("STS2_SEED_ENTRY_ORDINAL is not in the plan"))?,
            None => plan
                .entries
                .first()
                .ok_or_else(|| String::from("STS2_SEED_PLAN_JSON has no entries"))?,
        };
        let operation_id = required_identity("STS2_SEED_OPERATION_ID")?;
        let run_mode = env_or_default("STS2_SEED_RUN_MODE", "seeded_training")?;
        if !matches!(
            run_mode.as_str(),
            "seeded_training" | "seeded_replay" | "diagnostic"
        ) {
            return Err(String::from(
                "STS2_SEED_RUN_MODE must be seeded_training, seeded_replay, or diagnostic",
            ));
        }
        let verify_idempotency = match std::env::var("STS2_SEED_VERIFY_IDEMPOTENCY") {
            Ok(value) if value == "true" => true,
            Ok(value) if value == "false" => false,
            Ok(_) => {
                return Err(String::from(
                    "STS2_SEED_VERIFY_IDEMPOTENCY must be exactly true or false",
                ));
            }
            Err(std::env::VarError::NotPresent) => false,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(String::from(
                    "STS2_SEED_VERIFY_IDEMPOTENCY is not valid UTF-8",
                ));
            }
        };
        let context_raw = required("STS2_SEED_CONTEXT_JSON")?;
        if context_raw.len() > MAX_PLAN_BYTES {
            return Err(String::from(
                "STS2_SEED_CONTEXT_JSON exceeds its size limit",
            ));
        }
        let context: SelectedContext = serde_json::from_str(&context_raw)
            .map_err(|error| format!("STS2_SEED_CONTEXT_JSON is invalid: {error}"))?;
        validate_context(&context)?;
        let context_digest = context_digest(&context)?;
        if context_digest != context.context_digest {
            return Err(String::from(
                "STS2_SEED_CONTEXT_JSON context_digest does not match its fields",
            ));
        }
        if let Ok(expected) = std::env::var("STS2_SEED_CONTEXT_DIGEST")
            && expected != context_digest
        {
            return Err(String::from(
                "STS2_SEED_CONTEXT_DIGEST does not match the context",
            ));
        }
        Ok(Some(Self {
            plan_digest,
            entry_ordinal: entry.ordinal,
            requested_seed: entry.requested_seed.clone(),
            operation_id,
            run_mode,
            verify_idempotency,
            context,
        }))
    }

    pub(crate) fn selected_context(&self) -> Value {
        serde_json::to_value(&self.context).unwrap_or(Value::Null)
    }

    pub(crate) fn start_arguments(
        &self,
        instance_id: &str,
        mcp_session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        generation: u64,
    ) -> Value {
        json!({
            "instance_id": instance_id,
            "mcp_session_id": mcp_session_id,
            "lease_id": lease_id,
            "lease_epoch": lease_epoch,
            "generation": generation,
            "operation_id": self.operation_id,
            "seed": self.requested_seed,
            "run_mode": self.run_mode,
            "selected_context": self.selected_context(),
        })
    }

    pub(crate) fn context_digest(&self) -> &str {
        &self.context.context_digest
    }
}
