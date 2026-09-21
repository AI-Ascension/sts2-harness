// SPDX-License-Identifier: MIT

use super::*;
use std::path::Path;
use std::time::Duration;
use sts2_harness::ExoProcessConfig;

pub(crate) struct LookupAgentSettings {
    pub(crate) revision: String,
    pub(crate) timeout: Duration,
    pub(crate) owner_config_sha256: String,
    pub(crate) bootstrap_profile: bool,
    pub(crate) history_profile: bool,
}

pub(super) fn settings_from_environment(
    runner: EpisodeRunnerConfig,
) -> Result<RuntimeV3Settings, String> {
    if optional("STS2_COMBAT_DEMO")?.as_deref() == Some("true") {
        return Err(String::from(
            "game-information lookup mode cannot run the combat demo",
        ));
    }
    let (process, revision, timeout) = lookup_agent_from_environment()?;
    let owner_config_sha256 = required("STS2_LOOKUP_OWNER_CONFIG_SHA256")?;
    let bootstrap_profile =
        optional("STS2_EXO_LOOKUP_BOOTSTRAP")?.is_some_and(|value| value == "1" || value == "true");
    let history_profile =
        optional("STS2_EXO_LOOKUP_HISTORY")?.is_some_and(|value| value == "1" || value == "true");
    if !valid_sha256(&owner_config_sha256) {
        return Err(String::from(
            "STS2_LOOKUP_OWNER_CONFIG_SHA256 must be a lowercase SHA256 digest",
        ));
    }
    let exo = ExoConfig::new(
        revision.clone(),
        DEFAULT_MAX_REQUEST_BYTES,
        DEFAULT_MAX_RESPONSE_BYTES,
        timeout
            .as_millis()
            .try_into()
            .map_err(|_| String::from("lookup-agent timeout is too large"))?,
    )
    .map_err(|error| format!("lookup-agent runtime configuration is invalid: {error}"))?
    .with_visible_seed_forwarding(false);
    // The lookup lane names no decision provider, so its live-episode declaration is its whole
    // claim; installed last, once the run is admitted.
    live_admission::install(live_admission::resolve_declared(live_admission::declared()?))?;
    Ok(RuntimeV3Settings {
        runner,
        exo,
        process,
        admission: ExoRuntimeAdmission::legacy(),
        lifecycle: None,
        lookup_agent: Some(LookupAgentSettings {
            revision,
            timeout,
            owner_config_sha256,
            bootstrap_profile,
            history_profile,
        }),
    })
}

pub(super) fn lookup_agent_from_environment() -> Result<(ExoProcessConfig, String, Duration), String>
{
    let executable = required("STS2_LOOKUP_AGENT_BINARY")?;
    let arguments = string_list("STS2_LOOKUP_AGENT_ARGS_JSON")?;
    let working_directory = optional("STS2_LOOKUP_AGENT_WORKDIR")?;
    let inherited_environment = string_list("STS2_LOOKUP_AGENT_INHERITED_ENV_JSON")?;
    let process = lookup_process_config(
        executable.clone(),
        arguments,
        working_directory,
        inherited_environment,
    )?;
    let revision = required("STS2_LOOKUP_AGENT_SHA256")?;
    if !valid_sha256(&revision) {
        return Err(String::from(
            "STS2_LOOKUP_AGENT_SHA256 must be a lowercase SHA256 digest",
        ));
    }
    verify_lookup_agent_binary(Path::new(process.executable()), &revision)?;
    let timeout_millis = number("STS2_LOOKUP_AGENT_TIMEOUT_MILLIS", 120_000_u64)?;
    if timeout_millis == 0 || timeout_millis > 120_000 {
        return Err(String::from(
            "STS2_LOOKUP_AGENT_TIMEOUT_MILLIS must be between 1 and 120000",
        ));
    }
    Ok((process, revision, Duration::from_millis(timeout_millis)))
}

fn lookup_process_config(
    executable: String,
    arguments: Vec<String>,
    working_directory: Option<String>,
    inherited_environment: Vec<String>,
) -> Result<ExoProcessConfig, String> {
    if inherited_environment
        .iter()
        .any(|name| protected_lookup_environment(name))
    {
        return Err(String::from(
            "lookup-agent environment allowlist contains a protected runtime secret",
        ));
    }
    ExoProcessConfig::new(
        executable,
        arguments,
        working_directory,
        inherited_environment,
    )
    .map_err(|error| format!("lookup-agent process configuration is invalid: {error}"))
}

fn protected_lookup_environment(name: &str) -> bool {
    name == "STS2_LOOKUP_OWNER_CONFIG"
        || name == "STS2_LOOKUP_OWNER_CONFIG_SHA256"
        || name == "STS2_LOOKUP_CORPUS_STORE_KEY_HEX"
        || name == "STS2_LOOKUP_POLICY_STORE_KEY_HEX"
        || name == "STS2_LOOKUP_ARCHIVE_STORE_KEY_HEX"
        || name == "STS2_LOOKUP_BINDING_DISCOVERY_REQUEST_JSON"
        || name.starts_with("STS2_LOOKUP_OWNER_")
        || name.starts_with("STS2_WORKFLOW_TOKEN_")
}

pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

fn verify_lookup_agent_binary(path: &Path, expected_sha256: &str) -> Result<(), String> {
    use std::io::Read;

    if !path.is_absolute() {
        return Err(String::from(
            "STS2_LOOKUP_AGENT_BINARY must be an absolute path",
        ));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| String::from("lookup-agent executable cannot be inspected"))?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.len() > 128 * 1024 * 1024
    {
        return Err(String::from(
            "lookup-agent executable must be a bounded regular nonsymlink file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(String::from("lookup-agent executable is not executable"));
        }
    }
    let file = std::fs::File::open(path)
        .map_err(|_| String::from("lookup-agent executable cannot be opened"))?;
    let mut bytes = Vec::new();
    file.take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| String::from("lookup-agent executable cannot be hashed"))?;
    if bytes.len() > 128 * 1024 * 1024 || sts2_harness::sha256_hex(&bytes) != expected_sha256 {
        return Err(String::from(
            "lookup-agent executable does not match STS2_LOOKUP_AGENT_SHA256",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn lookup_agent_process_configuration_rejects_protected_runtime_environment() {
        for name in [
            "STS2_LOOKUP_CORPUS_STORE_KEY_HEX",
            "STS2_LOOKUP_POLICY_STORE_KEY_HEX",
            "STS2_LOOKUP_ARCHIVE_STORE_KEY_HEX",
            "STS2_LOOKUP_BINDING_DISCOVERY_REQUEST_JSON",
            "STS2_LOOKUP_OWNER_CONFIG",
            "STS2_WORKFLOW_TOKEN_LOOKUP_OWNER",
        ] {
            assert!(
                lookup_process_config(
                    String::from("/bin/true"),
                    Vec::new(),
                    None,
                    vec![String::from(name)],
                )
                .is_err(),
                "lookup-agent must not inherit protected runtime input {name}"
            );
        }
        let process = lookup_process_config(
            String::from("/bin/true"),
            Vec::new(),
            None,
            vec![String::from("STS2_LOOKUP_AGENT_LOG_LEVEL")],
        )
        .expect("nonsecret allowlist entry");
        assert_eq!(
            process.inherited_environment(),
            &["STS2_LOOKUP_AGENT_LOG_LEVEL"]
        );
    }
}
