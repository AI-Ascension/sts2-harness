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
use sts2_harness::exo_lookup_process::ExoLookupProfile;
#[cfg(target_os = "linux")]
use sts2_harness::parse_bridge_request_envelope;

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
