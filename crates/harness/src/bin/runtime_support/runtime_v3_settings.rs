// SPDX-License-Identifier: MIT

use sts2_harness::{
    EpisodeRunnerConfig, ExoConfig, ExoProcessConfig, RecoveryController, StabilityBarrier,
};

const REVIEWED_EXO_REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";
const DEFAULT_MAX_REQUEST_BYTES: usize = 128 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024;
const DEFAULT_TIMEOUT_MILLIS: u32 = 120_000;

pub(super) struct RuntimeV3Settings {
    pub(super) runner: EpisodeRunnerConfig,
    pub(super) exo: ExoConfig,
    pub(super) process: ExoProcessConfig,
}

impl RuntimeV3Settings {
    pub(super) fn from_environment() -> Result<Self, String> {
        let exo = exo_from_environment()?;
        let process = ExoProcessConfig::new(
            required("STS2_EXO_BRIDGE_BINARY")?,
            string_list("STS2_EXO_BRIDGE_ARGS_JSON")?,
            optional("STS2_EXO_BRIDGE_WORKDIR")?,
            string_list("STS2_EXO_INHERITED_ENV_JSON")?,
        )
        .map_err(|error| format!("Exo bridge process configuration is invalid: {error}"))?;
        let runner = runner_from_environment()?;
        Ok(Self {
            runner,
            exo,
            process,
        })
    }
}

fn verify_revision(revision: &str) -> Result<(), String> {
    let local_bridge = optional("STS2_PROVIDER_KIND")?.as_deref() == Some("ollama");
    if local_bridge
        && (revision.len() != 64 || optional("STS2_COMBAT_DEMO")?.as_deref() != Some("true"))
    {
        return Err(String::from(
            "Ollama demo requires the bridge SHA256 and explicit combat demo mode",
        ));
    }
    if local_bridge {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let file = std::fs::File::open(required("STS2_EXO_BRIDGE_BINARY")?)
            .map_err(|_| String::from("cannot open Ollama bridge for digest verification"))?;
        let mut bytes = Vec::new();
        file.take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| String::from("cannot hash Ollama bridge"))?;
        if bytes.len() > 128 * 1024 * 1024
            || format!("{:x}", Sha256::digest(&bytes)) != revision
            || !string_list("STS2_EXO_BRIDGE_ARGS_JSON")?.is_empty()
        {
            return Err(String::from(
                "Ollama bridge digest or arguments do not match",
            ));
        }
    }
    if !local_bridge && revision != REVIEWED_EXO_REVISION {
        return Err(String::from(
            "STS2_EXO_REVISION is not the reviewed Exo revision",
        ));
    }
    Ok(())
}

fn exo_from_environment() -> Result<ExoConfig, String> {
    let revision = required("STS2_EXO_REVISION")?;
    verify_revision(&revision)?;
    let forward_visible_seed = flag("STS2_EXO_FORWARD_VISIBLE_SEED")?;
    ExoConfig::new(
        revision,
        number(
            "STS2_EXO_MAX_REQUEST_BYTES",
            DEFAULT_MAX_REQUEST_BYTES as u64,
        )?
        .try_into()
        .map_err(|_| String::from("STS2_EXO_MAX_REQUEST_BYTES is too large"))?,
        number(
            "STS2_EXO_MAX_RESPONSE_BYTES",
            DEFAULT_MAX_RESPONSE_BYTES as u64,
        )?
        .try_into()
        .map_err(|_| String::from("STS2_EXO_MAX_RESPONSE_BYTES is too large"))?,
        number("STS2_EXO_TIMEOUT_MILLIS", u64::from(DEFAULT_TIMEOUT_MILLIS))?
            .try_into()
            .map_err(|_| String::from("STS2_EXO_TIMEOUT_MILLIS is too large"))?,
    )
    .map(|config| config.with_visible_seed_forwarding(forward_visible_seed))
    .map_err(|error| format!("Exo configuration is invalid: {error}"))
}

fn runner_from_environment() -> Result<EpisodeRunnerConfig, String> {
    let barrier = StabilityBarrier::new(
        number("STS2_BARRIER_MAX_POLLS", 8)?
            .try_into()
            .map_err(|_| String::from("STS2_BARRIER_MAX_POLLS is too large"))?,
        number("STS2_BARRIER_WAIT_MILLIS", 1_000)?
            .try_into()
            .map_err(|_| String::from("STS2_BARRIER_WAIT_MILLIS is too large"))?,
    )
    .map_err(|error| format!("stability barrier is invalid: {error}"))?;
    let recovery = RecoveryController::new(
        number("STS2_RECOVERY_MAX_ATTEMPTS", 2)?
            .try_into()
            .map_err(|_| String::from("STS2_RECOVERY_MAX_ATTEMPTS is too large"))?,
    )
    .map_err(|error| format!("recovery controller is invalid: {error}"))?;
    EpisodeRunnerConfig::new(
        number("STS2_MAX_STEPS", 1_024)?
            .try_into()
            .map_err(|_| String::from("STS2_MAX_STEPS is too large"))?,
        barrier,
        recovery,
        required("STS2_OBJECTIVE")?,
        string_list("STS2_HARD_CONSTRAINTS_JSON")?,
    )
    .map_err(|error| format!("episode runner configuration is invalid: {error}"))
}

fn required(name: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Err(format!("{name} is required")),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn optional(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn number<T>(name: &str, default: T) -> Result<T, String>
where
    T: TryFrom<u64> + Copy,
{
    let value = match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => return Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => return Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(format!("{name} is not valid UTF-8"));
        }
    };
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{name} must be a nonnegative integer"))?;
    T::try_from(parsed).map_err(|_| format!("{name} is outside its numeric bound"))
}

fn flag(name: &str) -> Result<bool, String> {
    match std::env::var(name) {
        Ok(value) => parse_flag(name, &value),
        Err(std::env::VarError::NotPresent) => Ok(true),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn parse_flag(name: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{name} must be exactly true or false")),
    }
}

fn string_list(name: &str) -> Result<Vec<String>, String> {
    let text = match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        Ok(_) | Err(std::env::VarError::NotPresent) => return Ok(Vec::new()),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(format!("{name} is not valid UTF-8"));
        }
    };
    serde_json::from_str::<Vec<String>>(&text)
        .map_err(|_| format!("{name} must be a JSON array of strings"))
}

#[cfg(test)]
mod tests {
    use super::parse_flag;

    #[test]
    fn seed_forwarding_flag_accepts_only_exact_booleans() {
        assert_eq!(parse_flag("X", "true"), Ok(true));
        assert_eq!(parse_flag("X", "false"), Ok(false));
        for value in ["", "1", "0", "yes", "TRUE", "True", " true"] {
            assert!(
                parse_flag("X", value).is_err(),
                "{value:?} must be rejected"
            );
        }
    }
}
