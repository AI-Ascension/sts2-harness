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
use std::time::Duration;

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
    closed: bool,
}

impl<F: LifecycleManifestFactory> ExoLifecycleRuntimeTransport<F> {
    pub fn new(
        owner: LifecycleOwner,
        store: Rc<RefCell<ExecutionStore>>,
        fingerprint: ExecutionFingerprint,
        effect: LifecycleProcessEffect,
        manifests: F,
    ) -> Self {
        Self {
            owner,
            store,
            fingerprint,
            effect,
            manifests,
            closed: false,
        }
    }

    fn exchange_inner(&mut self, bytes: &[u8]) -> Result<Vec<u8>, LifecycleError> {
        let envelope = parse_bridge_request_envelope(bytes, super::MAX_INPUT_BYTES)
            .map_err(|_| LifecycleError::Invalid)?;
        let manifest =
            self.manifests
                .manifest(&envelope.request_id, &envelope.turn_id, &envelope.request)?;
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
