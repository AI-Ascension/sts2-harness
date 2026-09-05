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
        let revision = required("STS2_EXO_REVISION")?;
        if revision != REVIEWED_EXO_REVISION {
            return Err(String::from(
                "STS2_EXO_REVISION is not the reviewed Exo revision",
            ));
        }
        let exo = ExoConfig::new(
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
        .map_err(|error| format!("Exo configuration is invalid: {error}"))?;
        let process = ExoProcessConfig::new(
            required("STS2_EXO_BRIDGE_BINARY")?,
            string_list("STS2_EXO_BRIDGE_ARGS_JSON")?,
            optional("STS2_EXO_BRIDGE_WORKDIR")?,
            string_list("STS2_EXO_INHERITED_ENV_JSON")?,
        )
        .map_err(|error| format!("Exo bridge process configuration is invalid: {error}"))?;
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
        let runner = EpisodeRunnerConfig::new(
            number("STS2_MAX_STEPS", 1_024)?
                .try_into()
                .map_err(|_| String::from("STS2_MAX_STEPS is too large"))?,
            barrier,
            recovery,
            required("STS2_OBJECTIVE")?,
            string_list("STS2_HARD_CONSTRAINTS_JSON")?,
        )
        .map_err(|error| format!("episode runner configuration is invalid: {error}"))?;
        Ok(Self {
            runner,
            exo,
            process,
        })
    }
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
