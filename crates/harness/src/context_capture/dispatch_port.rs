// SPDX-License-Identifier: MIT

//! The recording write port: the only place approved application bytes leave this module.

use super::dispatch_error::DispatchError;
use super::dispatch_material::PreparedBoundaryComponent;
use super::{CaptureBoundary, CaptureComponent, CapturePort, PreparedInput};
use std::fmt;

/// Exact materials handed to a write port.
///
/// The bytes are borrowed from the approved prepared input, so a port cannot receive a second
/// serialization of the same material.
#[derive(Clone, Copy, Debug)]
pub struct ApprovedDispatchMaterial<'a> {
    /// Identity of the held dispatch.
    pub dispatch_id: &'a str,
    /// Execution identity that owns the approved material.
    pub execution_id: &'a str,
    /// Attempt identity within the execution, when one was reserved.
    pub attempt_id: Option<&'a str>,
    /// Adapter id the material was prepared for.
    pub adapter_id: &'a str,
    /// Exact application boundary the port writes.
    pub boundary: CaptureBoundary,
    /// Digest of the ordered component bytes.
    pub approved_material_sha256: &'a str,
    /// Digest of the ordered manifest.
    pub manifest_sha256: &'a str,
    /// Ordered exact components.
    pub components: &'a [PreparedBoundaryComponent],
}

/// A provider-boundary write port.
///
/// A port receives approved application bytes exactly once and records what it wrote, so the
/// approved manifest can be compared with the observed one.
pub trait PreparedDispatchPort: fmt::Debug {
    /// Writes the approved material unchanged and returns the number of application bytes written.
    ///
    /// # Errors
    ///
    /// An error means the transport outcome is indeterminate rather than a clean failure, so the
    /// recorded receipt must never be retried blindly.
    fn write_prepared(
        &mut self,
        material: ApprovedDispatchMaterial<'_>,
    ) -> Result<usize, DispatchError>;
}

/// Bridges an approved dispatch to the harness capture port.
///
/// The capture port is the recording write port: it observes the same application bytes the
/// adapter dispatches, so an approved-versus-observed mismatch is detectable.  A capture port that
/// cannot record refuses the write, because exactness may not be claimed without a recording.
#[derive(Debug)]
pub struct CaptureRecordingPort<'a> {
    capture: &'a mut dyn CapturePort,
    write_attempts: u32,
}

impl<'a> CaptureRecordingPort<'a> {
    /// Wraps a capture port as the recording write port.
    pub fn new(capture: &'a mut dyn CapturePort) -> Self {
        Self {
            capture,
            write_attempts: 0,
        }
    }

    /// Number of application-boundary writes this port attempted.
    pub const fn write_attempts(&self) -> u32 {
        self.write_attempts
    }
}

impl PreparedDispatchPort for CaptureRecordingPort<'_> {
    fn write_prepared(
        &mut self,
        material: ApprovedDispatchMaterial<'_>,
    ) -> Result<usize, DispatchError> {
        self.write_attempts = self.write_attempts.saturating_add(1);
        if !self.capture.enabled() {
            return Err(DispatchError::CaptureDisabled);
        }
        let components: Vec<CaptureComponent<'_>> = material
            .components
            .iter()
            .map(|component| CaptureComponent {
                kind: component.kind,
                ordinal: component.ordinal,
                media_type: component.media_type.as_str(),
                bytes: component.bytes(),
            })
            .collect();
        self.capture
            .prepared_input(PreparedInput {
                execution_id: material.execution_id,
                attempt_id: material.attempt_id,
                boundary: material.boundary,
                components: &components,
            })
            .map_err(DispatchError::from)?;
        self.capture
            .write_completed_at(
                material.execution_id,
                material.attempt_id,
                material.boundary,
            )
            .map_err(DispatchError::from)?;
        Ok(components
            .iter()
            .map(|component| component.bytes.len())
            .sum())
    }
}
