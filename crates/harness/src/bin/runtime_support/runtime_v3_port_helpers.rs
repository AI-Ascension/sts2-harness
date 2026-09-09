// SPDX-License-Identifier: MIT

#[derive(Debug)]
pub(super) enum RuntimeV3ToolError {
    Transient(String),
    Terminal(String),
}

impl RuntimeV3ToolError {
    fn from_rpc_for(error: wire::RpcFailure, transient_allowed: bool) -> Self {
        if transient_allowed && error.is_transient() {
            Self::Transient(error.to_string())
        } else {
            Self::Terminal(error.to_string())
        }
    }

    pub(super) fn message(&self) -> &str {
        match self {
            Self::Transient(message) | Self::Terminal(message) => message,
        }
    }
}

fn classify_mcp_error(
    error: sts2_harness::PortError,
    transient_allowed: bool,
) -> RuntimeV3ToolError {
    if transient_allowed {
        RuntimeV3ToolError::Transient(error.to_string())
    } else {
        RuntimeV3ToolError::Terminal(error.to_string())
    }
}

fn finish_telemetry(telemetry: RuntimeV3Telemetry) {
    let report = telemetry.finish(std::time::Duration::from_secs(2));
    if report.export_status() != "delivered" {
        eprintln!(
            "runtime-v3 telemetry export status={} sent={} failed={} dropped={} timed_out={}",
            report.export_status(),
            report.sent,
            report.failed,
            report
                .normal_dropped
                .saturating_add(report.critical_dropped),
            report.timed_out
        );
    }
}
