// SPDX-License-Identifier: MIT

//! Explicit metadata-only capture around one normal bridge invocation. No extra inference.

use super::{Exchange, LIMIT, gate, options, record_profile};
use serde_json::Value;
use std::time::Instant;

#[path = "jev_capture_projection.rs"]
mod projection;
#[path = "jev_capture_store.rs"]
mod store;

type Failure = Box<dyn std::error::Error>;

pub(super) const SCHEMA: &str = "ascension.jev-redacted-capture.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Error {
    Input,
    Evidence,
    Storage,
    Quota,
    #[cfg(not(unix))]
    Platform,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Input => "capture_input_invalid",
            Self::Evidence => "capture_evidence_invalid",
            Self::Storage => "capture_storage_unavailable",
            Self::Quota => "capture_quota_exhausted",
            #[cfg(not(unix))]
            Self::Platform => "capture_platform_unsupported",
        })
    }
}

impl std::error::Error for Error {}

pub(super) fn run(
    bytes: &[u8],
    options: &options::Options,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Failure> {
    execute(bytes, options, &store::bridge_digest()?, exchange)
}

fn execute(
    bytes: &[u8],
    options: &options::Options,
    bridge_digest: &str,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Failure> {
    if bytes.len() > LIMIT || options.record {
        return Err(Error::Input.into());
    }
    let input: Value = serde_json::from_slice(bytes).map_err(|_| Error::Input)?;
    let identity = projection::Identity::new(&input, options, bridge_digest)?;
    let directory = options.audit_dir.as_deref().ok_or(Error::Input)?;
    let reservation = store::Reservation::new(directory, &identity.pending())?;
    let mut attempts = 0_u32;
    let started = Instant::now();
    let result = {
        let mut counted = |body: &[u8]| {
            // A second call would violate this capture profile, even after a transport error.
            if attempts != 0 {
                return Err(Error::Evidence.into());
            }
            attempts += 1;
            exchange(body)
        };
        record_profile(
            bytes,
            &options.model,
            gate(options),
            &mut counted,
            options.tactical,
            // The capture profile permits at most one transport invocation, so the ask never splits.
            false,
        )
    };
    let elapsed = started.elapsed().as_millis().min(9_007_199_254_740_991) as u64;
    finish(&reservation, &identity, result, attempts, elapsed)
}

fn finish(
    reservation: &store::Reservation,
    identity: &projection::Identity,
    result: Result<Value, Failure>,
    attempts: u32,
    elapsed: u64,
) -> Result<Value, Failure> {
    let record = match result {
        Ok(record) => record,
        Err(error) => {
            // Never store the error: transport errors may contain credentials or payloads.
            reservation.finish(&identity.failed(attempts, elapsed))?;
            return Err(error);
        }
    };
    let projected = match identity.complete(&record, attempts, elapsed) {
        Ok(projected) => projected,
        Err(error) => {
            reservation.finish(&identity.failed(attempts, elapsed))?;
            return Err(error.into());
        }
    };
    // Persist first. A write failure must not emit an action with missing capture evidence.
    reservation.finish(&projected)?;
    Ok(record["decision"].clone())
}

#[cfg(test)]
#[path = "jev_capture_tests.rs"]
mod tests;
