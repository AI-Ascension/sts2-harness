// SPDX-License-Identifier: MIT

mod config;
mod http;
mod mcp;
mod mcp_process;
mod response_validation;
mod v1_projection;
mod runtime_v3;
mod runtime_v3_parse;
mod runtime_v3_settings;
mod runtime_v3_wire;

pub(crate) use config::RuntimeConfig;

pub(crate) fn run(config: RuntimeConfig) -> Result<(), String> {
    if config.runtime_profile == "runtime-v3-gameplay" {
        runtime_v3::run(config)
    } else {
        mcp::run(config)
    }
}
