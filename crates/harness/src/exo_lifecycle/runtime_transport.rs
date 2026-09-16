// SPDX-License-Identifier: MIT

//! Runtime adapter that joins the lifecycle owner to the ordinary Exo transport port.

use super::{
    InvocationManifest, LifecycleError, LifecycleOwner, LifecycleProcessEffect, StartOutcome,
};
use crate::{
    ExecutionFingerprint, ExecutionStore, ExoTransport, ExoTransportError, encode_bridge_response,
    parse_bridge_request_envelope,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Supplies the manifest from the runtime's current, authoritative turn binding.
///
/// The factory is called after strict envelope parsing and before the durable owner starts. It
/// must not mint a manifest from provider-controlled request bytes alone.
pub trait LifecycleManifestFactory {
    fn manifest(
        &mut self,
        request_id: &str,
        turn_id: &str,
        request: &crate::ExoDecisionRequest,
    ) -> Result<InvocationManifest, LifecycleError>;
}

impl<F> LifecycleManifestFactory for F
where
    F: FnMut(&str, &str, &crate::ExoDecisionRequest) -> Result<InvocationManifest, LifecycleError>,
{
    fn manifest(
        &mut self,
        request_id: &str,
        turn_id: &str,
        request: &crate::ExoDecisionRequest,
    ) -> Result<InvocationManifest, LifecycleError> {
        self(request_id, turn_id, request)
    }
}

/// The only production bridge transport that can pass an admitted turn through a lifecycle owner.
///
/// It owns the same `ExecutionStore` incarnation that the runtime admitted and a concrete v2
/// process effect. The owner persists the provider reservation and send boundary before this type
/// can start the process; an ambiguous poll outcome is retained as Unknown by the owner.
pub struct ExoLifecycleRuntimeTransport<F> {
    owner: LifecycleOwner,
    store: Rc<RefCell<ExecutionStore>>,
    fingerprint: ExecutionFingerprint,
    effect: LifecycleProcessEffect,
    manifests: F,
    binding_ttl_seconds: u64,
    clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    closed: bool,
}

impl<F: LifecycleManifestFactory> ExoLifecycleRuntimeTransport<F> {
    pub fn new(
        owner: LifecycleOwner,
        store: Rc<RefCell<ExecutionStore>>,
        fingerprint: ExecutionFingerprint,
        effect: LifecycleProcessEffect,
        binding_ttl_seconds: u64,
        clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
        manifests: F,
    ) -> Result<Self, LifecycleError> {
        if binding_ttl_seconds == 0
            || binding_ttl_seconds > crate::provider_session::MAX_HISTORY_TTL_SECONDS
        {
            return Err(LifecycleError::Invalid);
        }
        Ok(Self {
            owner,
            store,
            fingerprint,
            effect,
            manifests,
            binding_ttl_seconds,
            clock,
            closed: false,
        })
    }

    fn exchange_inner(&mut self, bytes: &[u8]) -> Result<Vec<u8>, LifecycleError> {
        let envelope = parse_bridge_request_envelope(bytes, super::MAX_INPUT_BYTES)
            .map_err(|_| LifecycleError::Invalid)?;
        let mut manifest =
            self.manifests
                .manifest(&envelope.request_id, &envelope.turn_id, &envelope.request)?;
        manifest.input_digest = crate::sha256_hex(bytes);
        manifest.input_length = bytes.len();
        let manifest = if manifest.binding_id == "pending-binding" {
            let expires_at = binding_expiry((self.clock)(), self.binding_ttl_seconds)?;
            self.owner
                .prepare_one_shot_manifest(manifest, bytes, &expires_at)?
        } else {
            manifest
        };
        if manifest.execution_id != envelope.request.model_execution_id
            || manifest.request_id != envelope.request_id
            || manifest.host_turn_id != envelope.turn_id
            || manifest.authority.state_id != envelope.request.state_id
            || manifest.authority.generation != envelope.request.generation
        {
            return Err(LifecycleError::Stale);
        }
        let started = {
            let mut store = self
                .store
                .try_borrow_mut()
                .map_err(|_| LifecycleError::Busy)?;
            self.owner.start(
                manifest,
                bytes,
                &mut store,
                &self.fingerprint,
                &mut self.effect,
            )?
        };
        let decision = match started {
            StartOutcome::Stored(decision) => decision,
            StartOutcome::Started(mut inflight) => loop {
                let outcome = {
                    let mut store = self
                        .store
                        .try_borrow_mut()
                        .map_err(|_| LifecycleError::Busy)?;
                    self.owner.poll(&mut inflight, &mut store)?
                };
                if let Some(decision) = outcome {
                    break decision;
                }
                std::thread::sleep(Duration::from_millis(1));
            },
        };
        let value = serde_json::json!({
            "decision": "action",
            "action_id": decision.action_id,
            "rationale": decision.rationale,
            "confidence": decision.confidence,
        });
        let decision = serde_json::to_vec(&value).map_err(|_| LifecycleError::Invalid)?;
        encode_bridge_response(
            &envelope.request_id,
            &envelope.turn_id,
            crate::ExoWireOutcome::Decision,
            Some(&decision),
            None,
        )
        .map_err(|_| LifecycleError::Invalid)
    }
}

fn binding_expiry(now: SystemTime, ttl_seconds: u64) -> Result<String, LifecycleError> {
    if ttl_seconds == 0 || ttl_seconds > crate::provider_session::MAX_HISTORY_TTL_SECONDS {
        return Err(LifecycleError::Invalid);
    }
    let now = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| LifecycleError::Invalid)?;
    let now = i64::try_from(now.as_secs()).map_err(|_| LifecycleError::Invalid)?;
    let ttl = i64::try_from(ttl_seconds).map_err(|_| LifecycleError::Invalid)?;
    let expiry = now.checked_add(ttl).ok_or(LifecycleError::Invalid)?;
    let days = expiry.div_euclid(86_400);
    let day_seconds = expiry.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days).ok_or(LifecycleError::Invalid)?;
    if !(0..=9999).contains(&year) {
        return Err(LifecycleError::Invalid);
    }
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn civil_from_days(days: i64) -> Option<(i64, i64, i64)> {
    let shifted = days.checked_add(719_468)?;
    let era = if shifted >= 0 {
        shifted / 146_097
    } else {
        (shifted - 146_096) / 146_097
    };
    let day_of_era = shifted.checked_sub(era.checked_mul(146_097)?)?;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era.checked_add(era.checked_mul(400)?)?;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    Some((year, month, day))
}

#[cfg(test)]
mod tests {
    use super::{LifecycleError, binding_expiry};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn binding_expiry_uses_injected_time_and_bounded_ttl() {
        let now = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        assert_eq!(
            binding_expiry(now, 60).expect("expiry"),
            "2026-09-21T14:14:20Z"
        );
        assert_eq!(binding_expiry(now, 0), Err(LifecycleError::Invalid));
    }
}

impl<F: LifecycleManifestFactory> ExoTransport for ExoLifecycleRuntimeTransport<F> {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        if self.closed {
            return Err(ExoTransportError::Unavailable);
        }
        self.exchange_inner(request).map_err(|error| match error {
            LifecycleError::Invalid | LifecycleError::Stale => ExoTransportError::MalformedResponse,
            LifecycleError::Unknown | LifecycleError::Fenced => ExoTransportError::Unavailable,
            _ => ExoTransportError::Unavailable,
        })
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.closed = true;
        Ok(())
    }
}
