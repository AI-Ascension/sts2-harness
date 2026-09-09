// SPDX-License-Identifier: MIT

mod config;
mod http;
mod mcp;
mod mcp_process;
mod response_validation;
mod runtime_v3;
mod runtime_v3_parse;
mod runtime_v3_settings;
mod runtime_v3_telemetry;
mod runtime_v3_wire;
mod v1_projection;

pub(crate) use config::RuntimeConfig;

pub(crate) fn run(config: RuntimeConfig) -> Result<(), String> {
    if matches!(
        config.runtime_profile.as_str(),
        "runtime-v3-gameplay" | "runtime-v4-expert" | "runtime-v4-expert-rest-action"
    ) {
        runtime_v3::run(config)
    } else {
        mcp::run(config)
    }
}
