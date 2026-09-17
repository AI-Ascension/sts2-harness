// SPDX-License-Identifier: MIT

mod fixture;
mod http;
mod pins;
mod process;
mod proxy;

use fixture::Paths;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("exact-restore conformance failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = Arguments::parse(std::env::args().skip(1))?;
    pins::validate(&arguments.pins)?;
    std::fs::create_dir_all(&arguments.artifact_root)
        .map_err(|error| format!("create artifact root: {error}"))?;
    fixture::write_binary_provenance(&arguments.paths, &arguments.artifact_root)?;
    let mut failures = Vec::new();
    for outcome in ["positive", "refused", "unknown"] {
        eprintln!("running exact-restore outcome={outcome}");
        if let Err(error) = fixture::run_case(&arguments.paths, outcome, &arguments.artifact_root) {
            failures.push(format!("{outcome}: {error}"));
        }
    }
    if !failures.is_empty() {
        return Err(format!("outcome failures: {}", failures.join("; ")));
    }
    Ok(())
}

struct Arguments {
    pins: PathBuf,
    artifact_root: PathBuf,
    paths: Paths,
}

impl Arguments {
    fn parse(mut values: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut pins = None;
        let mut artifact_root = None;
        let mut paths = [None, None, None, None, None];
        while let Some(value) = values.next() {
            let (slot, name) = match value.as_str() {
                "--pins" => (&mut pins, "pins"),
                "--artifact-root" => (&mut artifact_root, "artifact-root"),
                "--harness-root" => (&mut paths[0], "harness-root"),
                "--harness-bin" => (&mut paths[1], "harness-bin"),
                "--mcp-bin" => (&mut paths[2], "mcp-bin"),
                "--gateway-bin" => (&mut paths[3], "gateway-bin"),
                "--mod-bin" => (&mut paths[4], "mod-bin"),
                other => return Err(format!("unknown argument {other}")),
            };
            *slot = Some(PathBuf::from(
                values
                    .next()
                    .ok_or_else(|| format!("{name} requires a path"))?,
            ));
        }
        let required = |slot: &mut Option<PathBuf>, name: &str| {
            slot.take().ok_or_else(|| format!("--{name} is required"))
        };
        Ok(Self {
            pins: required(&mut pins, "pins")?,
            artifact_root: artifact_root
                .unwrap_or_else(|| PathBuf::from("exact-restore-conformance-artifacts")),
            paths: Paths {
                harness_root: required(&mut paths[0], "harness-root")?,
                harness_bin: required(&mut paths[1], "harness-bin")?,
                mcp_bin: required(&mut paths[2], "mcp-bin")?,
                gateway_bin: required(&mut paths[3], "gateway-bin")?,
                mod_bin: required(&mut paths[4], "mod-bin")?,
            },
        })
    }
}
