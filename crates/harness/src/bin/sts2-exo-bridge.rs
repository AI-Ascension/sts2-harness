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
use sts2_harness::{EXO_SOURCE_REVISION, parse_bridge_request_envelope};

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
    if matches!(
        mode.as_str(),
        "--lookup-describe" | "--lookup-run" | "--lookup-synthetic"
    ) {
        let loaded = config::load_profile(path, true)?;
        if mode == "--lookup-describe" && rest.is_empty() {
            return write_output(
                &serde_json::to_vec(&loaded.lookup_description()?)
                    .map_err(|_| "exo_bridge_description")?,
            );
        }
        if mode == "--lookup-describe" || rest.len() != 1 || rest[0] != loaded.digest {
            return Err("exo_bridge_config_identity");
        }
        loaded.validate_route(mode == "--lookup-synthetic")?;
        return lookup::execute(&loaded, mode == "--lookup-synthetic");
    }
    if !matches!(mode.as_str(), "--describe" | "--run" | "--synthetic") {
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
    loaded.validate_route(mode == "--synthetic")?;
    let bytes = read_input()?;
    let envelope =
        parse_bridge_request_envelope(&bytes, 131_072).map_err(|_| "exo_bridge_invalid_request")?;
    let request = &envelope.request;
    if request.provider_revision != EXO_SOURCE_REVISION
        || request.map_context.is_some()
        || request.management_profile.is_some()
        || request.observation.get("protocol_version").is_some()
    {
        return Err("exo_bridge_unsupported_profile");
    }
    let response = run::execute(&loaded, envelope, mode == "--synthetic")?;
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
