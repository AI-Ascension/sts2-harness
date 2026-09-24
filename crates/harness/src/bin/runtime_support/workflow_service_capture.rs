// SPDX-License-Identifier: MIT

//! The served composition's configured boundary-capture surface.
//!
//! Which sink the served composition attaches, and what it retains, is an owner decision (`#398`).
//! The merged composition records through a bounded in-memory ring, and this module records that
//! decision on a configured surface without changing it: an unset surface keeps the recording ring
//! the served composition already attaches, so no production capture mode or retention default is
//! settled here that the merged composition did not already carry. An operator selects the mode and
//! the ring bounds with `STS2_WORKFLOW_CAPTURE_MODE`, `STS2_WORKFLOW_CAPTURE_RECORDS` and
//! `STS2_WORKFLOW_CAPTURE_BYTES`, and every unrecognised or contradictory value is refused at
//! startup rather than silently downgraded, so a misconfigured deployment cannot lose a boundary it
//! believes it recorded. See ADR 0070.

use sts2_harness::management::BoundaryCaptureSink;
use sts2_harness::{CaptureMode, MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS, MemoryCapture};

/// The capture decision the served composition attaches.
///
/// The mode and the two bounds are one value, so a configured bound can never be attached to a
/// different mode than the one it was resolved with. Bounds are zero only for `off`, whose sink
/// retains nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CaptureConfiguration {
    mode: CaptureMode,
    max_records: usize,
    max_content_bytes: usize,
}

impl CaptureConfiguration {
    /// The sink the served composition attaches for this decision.
    ///
    /// `off` attaches the sink that cannot record, so a served managed decision is refused before any
    /// provider write rather than published for a boundary nothing observed.
    pub(super) fn sink(&self) -> Result<BoundaryCaptureSink, String> {
        if self.mode == CaptureMode::Off {
            return Ok(BoundaryCaptureSink::disabled());
        }
        let capture = MemoryCapture::new(self.mode, self.max_records, self.max_content_bytes)
            .map_err(|error| format!("served boundary capture configuration: {error}"))?;
        Ok(BoundaryCaptureSink::new(Box::new(capture)))
    }
}

/// The sink the environment asks the served composition to attach.
pub(super) fn sink_from_environment() -> Result<BoundaryCaptureSink, String> {
    configuration_from_environment()?.sink()
}

/// The capture decision named by the environment.
fn configuration_from_environment() -> Result<CaptureConfiguration, String> {
    configuration_from_value(
        variable("STS2_WORKFLOW_CAPTURE_MODE")?,
        variable("STS2_WORKFLOW_CAPTURE_RECORDS")?,
        variable("STS2_WORKFLOW_CAPTURE_BYTES")?,
    )
}

/// One environment value, absent when the variable is unset.
fn variable(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

/// The capture decision for one observed environment surface.
///
/// An unset mode is the recording ring the served composition already attaches; `metadata` records
/// the ring's lifecycle and digests without the content bytes; `off` attaches the inert sink. A
/// bound that is non-numeric, zero, above its module maximum, or combined with `off` is refused.
fn configuration_from_value(
    mode: Option<String>,
    records: Option<String>,
    bytes: Option<String>,
) -> Result<CaptureConfiguration, String> {
    let mode = match mode {
        None => CaptureMode::Memory,
        Some(value) => match value.trim() {
            "memory" => CaptureMode::Memory,
            "metadata" => CaptureMode::Metadata,
            "off" => CaptureMode::Off,
            _ => {
                return Err(String::from(
                    "STS2_WORKFLOW_CAPTURE_MODE must be one of memory, metadata or off when set",
                ));
            }
        },
    };
    if mode == CaptureMode::Off {
        if records.is_some() || bytes.is_some() {
            return Err(String::from(
                "STS2_WORKFLOW_CAPTURE_RECORDS and STS2_WORKFLOW_CAPTURE_BYTES must be unset when \
                 STS2_WORKFLOW_CAPTURE_MODE is off",
            ));
        }
        return Ok(CaptureConfiguration {
            mode,
            max_records: 0,
            max_content_bytes: 0,
        });
    }
    Ok(CaptureConfiguration {
        mode,
        max_records: bound(
            records,
            "STS2_WORKFLOW_CAPTURE_RECORDS",
            MAX_CAPTURE_RECORDS,
        )?,
        max_content_bytes: bound(bytes, "STS2_WORKFLOW_CAPTURE_BYTES", MAX_CAPTURE_BYTES)?,
    })
}

/// One ring bound, defaulting to the module maximum when the variable is unset.
fn bound(value: Option<String>, name: &str, maximum: usize) -> Result<usize, String> {
    match value {
        None => Ok(maximum),
        Some(value) => {
            let message = format!("{name} must be an integer from 1 to {maximum} when set");
            let parsed = value.trim().parse::<usize>().map_err(|_| message.clone())?;
            if parsed == 0 || parsed > maximum {
                return Err(message);
            }
            Ok(parsed)
        }
    }
}

#[cfg(test)]
#[path = "workflow_service_capture_tests.rs"]
mod tests;
