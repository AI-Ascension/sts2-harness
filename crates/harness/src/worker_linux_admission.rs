// SPDX-License-Identifier: MIT

//! Native response completion joined to the worker-owned durable reservation.

use std::future::Future;

use crate::worker_handoff::WorkerExecutionReservation;
use crate::worker_local_linux::LinuxTransportError;
use crate::worker_runtime::{ResponseWriteStatus, WorkerRuntime, WorkerStartOutcome};

use super::LinuxWorkerExchange;

/// Response transport status and durable execution outcome are separate facts.
/// A written response is not proof that an episode or a game action completed.
pub struct LinuxAdmissionOutcome {
    pub response: ResponseWriteStatus,
    pub start: WorkerStartOutcome,
}

impl LinuxWorkerExchange<'_> {
    /// Admit the authenticated command and consume its original connection.
    /// Execution may start only after the correlated response is written.
    /// Dropping this future after admission conservatively retains uncertainty.
    pub async fn admit(self, runtime: &mut WorkerRuntime) -> Result<LinuxAdmissionOutcome, String> {
        let command = runtime.handle_authenticated(self.request())?;
        let (reply, reservation) = command.into_parts();
        let boot_id = runtime.worker_boot_id().to_owned();
        let pending = PendingReply {
            runtime,
            reservation,
        };
        pending.finish(self.write_reply(&boot_id, reply)).await
    }
}

struct PendingReply<'a> {
    runtime: &'a mut WorkerRuntime,
    reservation: Option<WorkerExecutionReservation>,
}

impl PendingReply<'_> {
    async fn finish(
        mut self,
        write: impl Future<Output = Result<(), LinuxTransportError>>,
    ) -> Result<LinuxAdmissionOutcome, String> {
        let response = match write.await {
            Ok(()) => ResponseWriteStatus::Written,
            Err(_) => ResponseWriteStatus::Failed,
        };
        let start = self
            .runtime
            .finish_reservation(self.reservation.take(), response)?;
        Ok(LinuxAdmissionOutcome { response, start })
    }
}

impl Drop for PendingReply<'_> {
    fn drop(&mut self) {
        if self.reservation.is_some() {
            // The runtime latches its admission fence before attempting the
            // durable UNKNOWN write. If storage is unavailable, that fence
            // remains closed; cancellation never authorizes another dispatch.
            let _ = self
                .runtime
                .finish_reservation(self.reservation.take(), ResponseWriteStatus::Failed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkerHandoffState;
    use crate::worker_frame_io::ConnectionDeadline;
    use crate::worker_linux_exchange::WorkerEndpointPolicy;
    use crate::worker_linux_exchange_tests::{config, support};
    use crate::worker_local_linux::AUTH_MAGIC;
    use crate::worker_runtime::tests::{admitted_reservation, handoff_state, request, runtime};
    use crate::worker_runtime_store::{try_lock, try_lock_recovery};
    use support::{Fixture, IDENTITY_DEADLINE, TestResult, read_frame, write_auth, write_frame};
    use tokio::net::UnixStream;

    #[tokio::test]
    async fn native_admission_starts_only_after_response_and_retains_failed_response() -> TestResult
    {
        for disconnected in [false, true] {
            let fixture = Fixture::new(b"worker-control-secret")?;
            let listener = config(&fixture)?.bind()?;
            let mut runtime = runtime().map_err(|error| error.to_string())?;
            let request = request().map_err(|error| error.to_string())?;
            let mut client = UnixStream::connect(&fixture.endpoint).await?;
            write_auth(&mut client, &fixture.secret, AUTH_MAGIC).await?;
            write_frame(&mut client, &serde_json::to_vec(request.fields())?).await?;
            let exchange = LinuxWorkerExchange::accept_with_policy(
                &listener,
                ConnectionDeadline::start(IDENTITY_DEADLINE)?,
                WorkerEndpointPolicy::WatchdogOwner,
            )
            .await?;
            let mut client = Some(client);
            if disconnected {
                drop(client.take());
            }
            let outcome = exchange.admit(&mut runtime).await?;
            let state = handoff_state(&runtime).map_err(|error| error.to_string())?;
            if disconnected {
                assert_eq!(outcome.response, ResponseWriteStatus::Failed);
                assert!(matches!(outcome.start, WorkerStartOutcome::Unknown { .. }));
                assert_eq!(state, WorkerHandoffState::Unknown);
            } else {
                assert_eq!(outcome.response, ResponseWriteStatus::Written);
                assert!(matches!(outcome.start, WorkerStartOutcome::Started(_)));
                assert_eq!(state, WorkerHandoffState::Running);
                let response: serde_json::Value = serde_json::from_slice(
                    &read_frame(client.as_mut().ok_or("missing client")?).await?,
                )?;
                assert_eq!(response["command"], "dispatch");
                assert_eq!(response["request_id"], request.fields()["request_id"]);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_response_guard_retains_unknown_before_or_after_first_poll()
    -> Result<(), Box<dyn std::error::Error>> {
        for polled in [false, true] {
            let mut runtime = runtime()?;
            let reservation = admitted_reservation(&mut runtime)?;
            let pending = PendingReply {
                runtime: &mut runtime,
                reservation: Some(reservation),
            };
            let mut response = Box::pin(pending.finish(std::future::pending()));
            if polled {
                std::future::poll_fn(|cx| {
                    assert!(response.as_mut().poll(cx).is_pending());
                    std::task::Poll::Ready(())
                })
                .await;
            }
            drop(response);
            assert_eq!(handoff_state(&runtime)?, WorkerHandoffState::Unknown);
            assert!(try_lock(runtime.store()).is_err());
        }
        Ok(())
    }

    #[test]
    fn cancelled_response_with_busy_store_keeps_admission_fenced()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = runtime()?;
        let reservation = admitted_reservation(&mut runtime)?;
        let shared = runtime.store().clone();
        let held = try_lock_recovery(&shared)?;
        drop(PendingReply {
            runtime: &mut runtime,
            reservation: Some(reservation),
        });
        drop(held);
        assert_eq!(handoff_state(&runtime)?, WorkerHandoffState::Admitted);
        assert!(try_lock(runtime.store()).is_err());
        Ok(())
    }
}
