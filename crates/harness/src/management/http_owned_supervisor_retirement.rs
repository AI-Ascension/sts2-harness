// SPDX-License-Identifier: MIT

//! Actual generation retirement and retained cleanup ownership.

use super::*;

impl Supervisor {
    pub(super) fn retire(&mut self) -> Result<(), HttpError> {
        let Some(current) = self.generation.as_mut() else {
            return Ok(());
        };
        current.retiring = true;
        let reaped = self
            .scope
            .terminate(&mut current.child, Instant::now() + REAP_BUDGET)
            .is_ok();
        let number = current.number;
        for (sequence, pending) in std::mem::take(&mut self.pending) {
            if pending.sent {
                let code = if reaped {
                    "dispatch_backend_terminated"
                } else {
                    "dispatch_termination_pending"
                };
                let error = match pending.effect {
                    RouteEffect::ReadOnly => ManagementError::unavailable(
                        code,
                        "read-only backend did not return an answer",
                    ),
                    RouteEffect::MayWrite => ManagementError::write_state_unknown(
                        code,
                        "backend outcome is unknown; recover the original request or operation identity before resubmission",
                    ),
                };
                self.error(sequence, error)?;
            } else {
                self.refuse(sequence, "dispatch_not_started")?;
            }
        }
        for (sequence, _) in std::mem::take(&mut self.diagnostics) {
            self.emit(FromBackend::DiagnosticLost {
                generation: self.parent_generation,
                sequence,
            })?;
        }
        self.emit(FromBackend::BackendRetired {
            generation: self.parent_generation,
            backend_generation: number,
            reaped,
        })?;
        if reaped {
            self.generation = None;
        }
        Ok(())
    }

    pub(in crate::management::http::owned_supervisor) fn drain_before_exit(&mut self) {
        // The helper remains the explicit owner if the kernel cannot yet prove exit. Closing
        // its parent channel cannot detach the backend or authorize a replacement generation.
        if let Some(current) = self.generation.as_mut() {
            while self
                .scope
                .terminate(&mut current.child, Instant::now() + REAP_BUDGET)
                .is_err()
            {
                thread::sleep(Duration::from_millis(10));
            }
        }
        while self.scope.drain(Instant::now() + REAP_BUDGET).is_err() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub(in crate::management::http::owned_supervisor) fn report_failure(&mut self, code: &str) {
        let frame = FromBackend::SupervisorFailed {
            generation: self.parent_generation,
            code: code.to_owned(),
        };
        if self.parent.queue(&frame).is_err() {
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(100);
        while !self.parent.output_empty() && Instant::now() < deadline {
            if self.parent.flush().is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn refuse(&mut self, sequence: u64, code: &'static str) -> Result<(), HttpError> {
        self.error(
            sequence,
            ManagementError::unavailable(code, "no additional management handler was started"),
        )
    }

    pub(super) fn error(&mut self, sequence: u64, error: ManagementError) -> Result<(), HttpError> {
        let response =
            response::error_response(HttpError::from_management(error), MAX_RESPONSE_BYTES)?;
        self.emit(FromBackend::Response {
            generation: self.parent_generation,
            sequence,
            status: response.status,
            body: response.body,
        })
    }

    pub(super) fn emit(&mut self, frame: FromBackend) -> Result<(), HttpError> {
        self.parent
            .queue(&frame)
            .map_err(|_| fault("owned_parent_bounds"))
    }
}
