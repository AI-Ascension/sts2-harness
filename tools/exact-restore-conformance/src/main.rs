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
    for outcome in ["positive", "refused", "unknown"] {
        eprintln!("running exact-restore outcome={outcome}");
        fixture::run_case(&arguments.paths, outcome)?;
    }
    Ok(())
}

struct Arguments {
    pins: PathBuf,
    paths: Paths,
}

impl Arguments {
    fn parse(mut values: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut pins = None;
        let mut paths = [None, None, None, None, None];
        while let Some(value) = values.next() {
            let (slot, name) = match value.as_str() {
                "--pins" => (&mut pins, "pins"),
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
