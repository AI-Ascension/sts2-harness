// SPDX-License-Identifier: MIT

use sts2_harness::{
    EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES, EpisodeRunnerConfig, ExoConfig,
    ExoProcessConfig, RecoveryController, StabilityBarrier,
};

use super::config::RuntimeConfig;

#[path = "../support/ollama_options.rs"]
mod ollama_options;

const REVIEWED_EXO_REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";
const DEFAULT_MAX_REQUEST_BYTES: usize = EXO_MAX_STANDARD_REQUEST_BYTES;
const DEFAULT_MAP_MAX_REQUEST_BYTES: usize = EXO_MAX_MAP_REQUEST_BYTES;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024;
const DEFAULT_TIMEOUT_MILLIS: u32 = 120_000;

pub(super) struct RuntimeV3Settings {
    pub(super) runner: EpisodeRunnerConfig,
    pub(super) exo: ExoConfig,
    pub(super) process: ExoProcessConfig,
}

impl RuntimeV3Settings {
    pub(super) fn from_environment(config: &RuntimeConfig) -> Result<Self, String> {
        let exo = exo_from_environment(config.map_context_enabled)?;
        let process = ExoProcessConfig::new(
            required("STS2_EXO_BRIDGE_BINARY")?,
            string_list("STS2_EXO_BRIDGE_ARGS_JSON")?,
            optional("STS2_EXO_BRIDGE_WORKDIR")?,
            string_list("STS2_EXO_INHERITED_ENV_JSON")?,
        )
        .map_err(|error| format!("Exo bridge process configuration is invalid: {error}"))?;
        let runner = runner_from_environment(config.map_context_enabled)?;
        Ok(Self {
            runner,
            exo,
            process,
        })
    }
}

fn verify_revision(revision: &str) -> Result<(), String> {
    let provider = optional("STS2_PROVIDER_KIND")?;
    let local_bridge = matches!(provider.as_deref(), Some("ollama" | "openai-astra"));
    let live_episode = optional("STS2_LIVE_EPISODE")?.as_deref() == Some("true");
    if live_episode && provider.as_deref() != Some("openai-astra") {
        return Err(String::from(
            "Live episode mode requires the OpenAI Astra provider",
        ));
    }
    if local_bridge
        && (revision.len() != 64
            || !(optional("STS2_COMBAT_DEMO")?.as_deref() == Some("true") || live_episode))
    {
        return Err(String::from(
            "Local provider requires the bridge SHA256 and explicit combat or live episode mode",
        ));
    }
    if local_bridge {
        use std::io::Read;
        let file = std::fs::File::open(required("STS2_EXO_BRIDGE_BINARY")?)
            .map_err(|_| String::from("cannot open provider bridge for digest verification"))?;
        let mut bytes = Vec::new();
        file.take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| String::from("cannot hash provider bridge"))?;
        if bytes.len() > 128 * 1024 * 1024
            || sts2_harness::sha256_hex(&bytes) != revision
            || !local_bridge_arguments_allowed(
                provider.as_deref(),
                &string_list("STS2_EXO_BRIDGE_ARGS_JSON")?,
            )
        {
            return Err(String::from(
                "Provider bridge digest or arguments do not match",
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

fn local_bridge_arguments_allowed(provider: Option<&str>, arguments: &[String]) -> bool {
    if arguments.is_empty() {
        return true;
    }
    if provider != Some("ollama") || arguments.len() != 2 || arguments[0] != "--model" {
        return false;
    }
    ollama_options::Options::parse(arguments.iter().cloned())
        .is_ok_and(|options| !options.describe && options.model == arguments[1])
}

fn exo_from_environment(map_context_enabled: bool) -> Result<ExoConfig, String> {
    let revision = required("STS2_EXO_REVISION")?;
    verify_revision(&revision)?;
    let forward_visible_seed = flag("STS2_EXO_FORWARD_VISIBLE_SEED")?;
    let default_max_request_bytes = if map_context_enabled {
        DEFAULT_MAP_MAX_REQUEST_BYTES
    } else {
        DEFAULT_MAX_REQUEST_BYTES
    };
    let max_request_bytes = number("STS2_EXO_MAX_REQUEST_BYTES", default_max_request_bytes)?;
    validate_request_bound(map_context_enabled, max_request_bytes)?;
    ExoConfig::new(
        revision,
        max_request_bytes,
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

fn validate_request_bound(
    map_context_enabled: bool,
    max_request_bytes: usize,
) -> Result<(), String> {
    let maximum = if map_context_enabled {
        EXO_MAX_MAP_REQUEST_BYTES
    } else {
        EXO_MAX_STANDARD_REQUEST_BYTES
    };
    if max_request_bytes == 0 || max_request_bytes > maximum {
        return Err(format!(
            "STS2_EXO_MAX_REQUEST_BYTES must be between 1 and {maximum}"
        ));
    }
    if map_context_enabled && max_request_bytes < EXO_MAX_MAP_REQUEST_BYTES {
        return Err(format!(
            "STS2_EXO_MAX_REQUEST_BYTES must be {EXO_MAX_MAP_REQUEST_BYTES} when STS2_ENABLE_MAP_CONTEXT is true"
        ));
    }
    Ok(())
}

fn runner_from_environment(map_context_enabled: bool) -> Result<EpisodeRunnerConfig, String> {
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
    .map(|config| config.with_map_context_enabled(map_context_enabled))
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
    use super::{
        EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES, parse_flag,
        validate_request_bound,
    };

    #[test]
    fn local_bridge_admission_allows_only_explicit_ollama_model_selection() {
        use super::local_bridge_arguments_allowed;
        let selected = vec!["--model".to_owned(), "team/custom:7b".to_owned()];
        assert!(local_bridge_arguments_allowed(Some("ollama"), &selected));
        assert!(!local_bridge_arguments_allowed(
            Some("openai-astra"),
            &selected
        ));
        assert!(!local_bridge_arguments_allowed(None, &selected));
        assert!(local_bridge_arguments_allowed(Some("ollama"), &[]));
        assert!(local_bridge_arguments_allowed(Some("openai-astra"), &[]));
        for args in [
            vec!["--describe"],
            vec!["--model", ""],
            vec!["--model", "--describe"],
            vec!["--model", "x", "--describe"],
            vec!["--endpoint", "example.invalid"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(!local_bridge_arguments_allowed(Some("ollama"), &args));
        }
    }

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

    #[test]
    fn map_opt_in_selects_derived_full_snapshot_request_bound() {
        assert_eq!(EXO_MAX_STANDARD_REQUEST_BYTES, 128 * 1024);
        assert_eq!(EXO_MAX_MAP_REQUEST_BYTES, 393_443);
        assert!(validate_request_bound(false, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());
        assert!(validate_request_bound(false, EXO_MAX_STANDARD_REQUEST_BYTES + 1).is_err());
        assert!(validate_request_bound(true, EXO_MAX_MAP_REQUEST_BYTES).is_ok());
        assert!(validate_request_bound(true, EXO_MAX_STANDARD_REQUEST_BYTES).is_err());
    }
}
