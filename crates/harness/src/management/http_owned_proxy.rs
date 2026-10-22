// SPDX-License-Identifier: MIT

use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{self, SyncSender};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::owned_deadline::Deadline;
use super::owned_supervisor::REAP_BUDGET;
use super::*;

pub(super) struct Work {
    pub(super) request: HttpRequest,
    pub(super) deadline: Deadline,
    pub(super) reply: SyncSender<Result<HttpResponse, HttpError>>,
}

#[derive(Clone)]
pub(super) struct Proxy {
    pub(super) work: SyncSender<Work>,
    pub(super) diagnostics: SyncSender<String>,
    pub(super) lost_diagnostics: Arc<AtomicU64>,
    pub(super) origin: Instant,
}

impl Proxy {
    pub(super) fn dispatch(
        &self,
        request: HttpRequest,
        budget: Duration,
    ) -> Result<HttpResponse, HttpError> {
        let deadline =
            Deadline::after(budget).ok_or_else(|| unavailable("owned_dispatch_clock"))?;
        let effect = route_effects::classify(&request);
        let (reply, receiver) = mpsc::sync_channel(1);
        self.work
            .try_send(Work {
                request,
                deadline,
                reply,
            })
            .map_err(|_| unavailable("dispatch_capacity_exhausted"))?;
        let wait = deadline.remaining() + REAP_BUDGET + Duration::from_millis(250);
        receiver
            .recv_timeout(wait)
            .unwrap_or_else(|_| Err(uncertain(effect, "dispatch_termination_pending")))
    }

    pub(super) fn log(&self, line: &str) {
        if line.len() > owned_wire::MAX_DIAGNOSTIC_BYTES
            || self.diagnostics.try_send(line.to_owned()).is_err()
        {
            lost(&self.lost_diagnostics);
        }
    }
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedManagementProxy")
            .finish_non_exhaustive()
    }
}

impl lifecycle_log::LifecycleLogSink for Proxy {
    fn wall_clock(&self) -> Option<(u64, u32)> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|value| (value.as_secs(), value.subsec_nanos()))
    }
    fn monotonic_millis(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn write_line(&self, line: &str) {
        self.log(line);
    }
}

impl ManagementFailurePort for Proxy {
    fn report(&self, line: &str) {
        self.log(line);
    }
}

pub(super) fn lost(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_add(1))
    });
}

pub(super) fn unavailable(code: &'static str) -> HttpError {
    HttpError::from_management(ManagementError::unavailable(
        code,
        "no additional management handler was started",
    ))
}

pub(super) fn uncertain(effect: route_effects::RouteEffect, code: &'static str) -> HttpError {
    HttpError::from_management(match effect {
        route_effects::RouteEffect::ReadOnly => ManagementError::unavailable(
            code,
            "read-only management backend did not return an answer",
        ),
        route_effects::RouteEffect::MayWrite => ManagementError::write_state_unknown(
            code,
            "backend outcome is unknown; recover the original request or operation identity before resubmission",
        ),
    })
}
