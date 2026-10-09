// SPDX-License-Identifier: MIT

//! Explicit single-turn Exo process entrypoint. Full episode admission remains separately gated.

#[cfg(target_os = "linux")]
use sts2_harness::exo_bridge_configuration as config;
#[path = "support/exo_bridge_lookup.rs"]
#[cfg(target_os = "linux")]
mod lookup;
#[path = "support/exo_bridge_run.rs"]
#[cfg(target_os = "linux")]
mod run;

#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use sts2_harness::ExoProcessConfig;
#[cfg(target_os = "linux")]
use sts2_harness::exo_bridge_configuration::SyntheticLoopbackInspection;
#[cfg(target_os = "linux")]
use sts2_harness::exo_lookup_process::ExoLookupProfile;
#[cfg(target_os = "linux")]
use sts2_harness::parse_bridge_request_envelope;
#[cfg(target_os = "linux")]
use sts2_harness::provider_session::NativeCapabilities;

#[cfg(target_os = "linux")]
const MAX_PROVIDER_CAPABILITIES_BYTES: usize = 8 * 1024;

#[cfg(target_os = "linux")]
fn main() {
    if let Err(code) = execute() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("exo_bridge_platform_unsupported");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn execute() -> Result<(), &'static str> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let [mode, path, rest @ ..] = args.as_slice() else {
        return Err("exo_bridge_arguments");
    };
    if mode == "--synthetic-provider-capabilities" {
        let [digest, instance_id] = rest else {
            return Err("exo_bridge_arguments");
        };
        return describe_synthetic_provider_capabilities(path, digest, instance_id);
    }
    if mode == "--provider-capabilities" {
        return describe_provider_capabilities(path, provider_capability_instance(rest)?);
    }
    if let Some(profile) = lookup::profile_for(mode) {
        let loaded = config::load_profile(path, true)?;
        let describe = mode.ends_with("-describe");
        if describe && rest.is_empty() {
            let description = match profile {
                ExoLookupProfile::Terminal => loaded.lookup_description()?,
                ExoLookupProfile::Bootstrap => loaded.lookup_bootstrap_description()?,
                ExoLookupProfile::History => loaded.lookup_history_description()?,
            };
            return write_output(
                &serde_json::to_vec(&description).map_err(|_| "exo_bridge_description")?,
            );
        }
        if describe || rest.len() != 1 || rest[0] != loaded.digest {
            return Err("exo_bridge_config_identity");
        }
        let synthetic = mode.ends_with("-synthetic");
        loaded.validate_route(synthetic)?;
        return lookup::execute_profile(&loaded, synthetic, profile);
    }
    if !matches!(
        mode.as_str(),
        "--describe" | "--run" | "--synthetic" | "--run-v2" | "--synthetic-v2"
    ) {
        return Err("exo_bridge_arguments");
    }
    let loaded = config::load(path)?;
    if mode == "--describe" {
        if !rest.is_empty() {
            return Err("exo_bridge_arguments");
        }
        let bytes =
            serde_json::to_vec(&loaded.description()?).map_err(|_| "exo_bridge_description")?;
        return write_output(&bytes);
    }
    if rest.len() != 1 || rest[0] != loaded.digest {
        return Err("exo_bridge_config_identity");
    }
    loaded.validate_route(mode == "--synthetic" || mode == "--synthetic-v2")?;
    let bytes = read_input()?;
    let envelope =
        parse_bridge_request_envelope(&bytes, 131_072).map_err(|_| "exo_bridge_invalid_request")?;
    if config::unsupported_profile_axis(&envelope.request).is_some() {
        return Err(config::UNSUPPORTED_PROFILE_CODE);
    }
    let response = if mode == "--run-v2" || mode == "--synthetic-v2" {
        run::execute_v2(&loaded, envelope, mode == "--synthetic-v2")?
    } else {
        run::execute(&loaded, envelope, mode == "--synthetic")?
    };
    write_output(&response)
}

#[cfg(target_os = "linux")]
fn describe_provider_capabilities(path: &str, instance_id: &str) -> Result<(), &'static str> {
    let loaded = config::load(path)?;
    if loaded.guarded_private_state().is_none() {
        return Err("exo_bridge_private_policy");
    }
    loaded.validate_route(false)?;
    let bridge = std::env::current_exe().map_err(|_| "exo_bridge_package")?;
    let identity = loaded.inspected_identity(&bridge, instance_id)?;
    let capabilities = NativeCapabilities::reviewed_exo_one_shot(&identity)
        .map_err(|_| "exo_bridge_provider_profile")?;
    write_output(&serialize_provider_capabilities(&capabilities)?)
}

#[cfg(target_os = "linux")]
fn describe_synthetic_provider_capabilities(
    path: &str,
    digest: &str,
    instance_id: &str,
) -> Result<(), &'static str> {
    let loaded = config::load(path)?;
    let executable = std::env::current_exe().map_err(|_| "exo_bridge_package")?;
    let process = ExoProcessConfig::new(
        executable.to_string_lossy().into_owned(),
        vec![
            String::from("--synthetic"),
            path.to_owned(),
            digest.to_owned(),
        ],
        None,
        Vec::new(),
    )
    .map_err(|_| "exo_bridge_provider_profile")?;
    let inspection =
        SyntheticLoopbackInspection::inspect(process, &loaded.config.executor, instance_id)
            .map_err(|_| "exo_bridge_provider_profile")?;
    let capabilities = NativeCapabilities::reviewed_synthetic_exo_one_shot(&inspection)
        .map_err(|_| "exo_bridge_provider_profile")?;
    write_output(&serialize_provider_capabilities(&capabilities)?)
}

#[cfg(target_os = "linux")]
fn provider_capability_instance(rest: &[String]) -> Result<&str, &'static str> {
    match rest {
        [instance_id] => Ok(instance_id),
        _ => Err("exo_bridge_arguments"),
    }
}

#[cfg(target_os = "linux")]
fn serialize_provider_capabilities(
    capabilities: &NativeCapabilities,
) -> Result<Vec<u8>, &'static str> {
    let bytes = serde_json::to_vec(capabilities).map_err(|_| "exo_bridge_description")?;
    if bytes.len() > MAX_PROVIDER_CAPABILITIES_BYTES {
        return Err("exo_bridge_description_bound");
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn write_output(bytes: &[u8]) -> Result<(), &'static str> {
    let mut output = std::io::stdout();
    output
        .write_all(bytes)
        .and_then(|()| output.flush())
        .map_err(|_| "exo_bridge_output")
}

#[cfg(target_os = "linux")]
fn read_input() -> Result<Vec<u8>, &'static str> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    // A missing EOF has a finite process lifetime; main exits on timeout without awaiting stdin.
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = std::io::stdin()
            .take(131_073)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "exo_bridge_input_timeout")?
        .map_err(|_| "exo_bridge_input")
}

#[cfg(all(test, target_os = "linux"))]
mod provider_capabilities_tests {
    use super::*;

    #[test]
    fn descriptor_mode_requires_one_argument_and_bounds_output() {
        assert_eq!(
            provider_capability_instance(&[String::from("instance")]),
            Ok("instance")
        );
        assert_eq!(
            provider_capability_instance(&[]),
            Err("exo_bridge_arguments")
        );
        let mut capabilities = NativeCapabilities::fixture();
        capabilities.unknown_methods = "x".repeat(MAX_PROVIDER_CAPABILITIES_BYTES);
        assert_eq!(
            serialize_provider_capabilities(&capabilities),
            Err("exo_bridge_description_bound")
        );
    }
}
