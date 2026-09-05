// SPDX-License-Identifier: MIT

mod config;
mod http;
mod mcp;
mod mcp_process;
mod response_validation;
mod v1_projection;

pub(crate) use config::RuntimeConfig;
pub(crate) use mcp::run;
