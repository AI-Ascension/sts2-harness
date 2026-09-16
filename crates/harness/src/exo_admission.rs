// SPDX-License-Identifier: MIT

//! Reviewed admission boundary for the runtime Exo transport seam.
//!
//! The runtime must not give a model provider a transport until the offline, model-free contract
//! [`preflight`] has admitted the configured deployment. [`ExoAdmissionPlan`] assembles the
//! reviewed capability descriptor whose deployment identity is the identity *inspected* from the
//! deployment actually on disk, and cross-checks it against the operator-trusted configuration. It
//! refuses the run when a pinned identity axis was not inspected or its inspected bytes do not match
//! the pin, when a required capability is missing or unverified, or when the advertised schema,
//! runtime, platform or profile is unsupported, and only then admits one correlated turn through
//! [`ExoAdmittedTransport`]. Every refusal is produced before the inner transport is dispatched, so
//! a rejected deployment cannot cause a model or game effect.
//!
//! [`ExoRuntimeAdmission`] is the production-facing decision. `Enveloped` is the reviewed,
//! fail-closed default that speaks the versioned `sts2.exo-bridge-wire-v1` envelope. `Legacy` is
//! an explicit operator acknowledgement of the un-admitted raw-wire process bridge used by the
//! local `ollama`, `openai-astra` and `synthetic` fixtures, which cannot accept that envelope.

use std::path::Path;

use crate::ExoCapabilityState;
use crate::exo::{
    EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoCapabilityDescriptor, ExoIdentity,
    ExoIdentityError, ExoPreflightError, ExoTransport, ExoTransportError, ExoTrustedConfiguration,
    preflight,
};
use crate::exo_admitted_transport::{ExoAdmissionError, ExoAdmittedTransport};
use crate::exo_lifecycle::{ExoLifecycleRuntimeTransport, LifecycleManifestFactory};
use crate::sha256_hex;

/// The exact artifact bytes an operator inspected for one deployment.
///
/// Every digest axis is derived from the bytes recorded here, so replacing a package, extension,
/// bridge, prompt, tool or configuration artifact changes the inspected identity and can no longer
/// satisfy the operator's pin. `None` records an axis the inspection did not observe; [`preflight`]
/// then refuses it, because an unobserved axis cannot be bound to the pin.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExoInspectedArtifacts {
    pub package: Option<Vec<u8>>,
    pub extension: Option<Vec<u8>>,
    pub bridge: Option<Vec<u8>>,
    pub prompt: Option<Vec<u8>>,
    pub tool: Option<Vec<u8>>,
    pub config: Option<Vec<u8>>,
}

impl ExoInspectedArtifacts {
    /// The largest executable this seam hashes, shared with the reviewed Exo bridge loader.
    pub const MAX_INSPECTED_ARTIFACT_BYTES: u64 =
        crate::exo_bridge_configuration::MAX_EXECUTOR_BYTES as u64;

    /// Reads one artifact's exact bytes, so replacing the file changes the inspected digest.
    ///
    /// The read is bounded to [`Self::MAX_INSPECTED_ARTIFACT_BYTES`], so an oversized artifact
    /// is an error rather than an unbounded allocation. Extension and configuration files use their
    /// smaller, artifact-specific bounds in the shared bridge loader.
    pub fn read(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
        use std::io::Read as _;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(Self::MAX_INSPECTED_ARTIFACT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > Self::MAX_INSPECTED_ARTIFACT_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "inspected artifact exceeds the maximum hashed size",
            ));
        }
        Ok(bytes)
    }

    /// The identity observed from the deployment actually on disk: every digest axis is computed
    /// from the inspected artifact bytes and the source revision is the harness-reviewed pin.
    ///
    /// An axis with no inspected bytes stays `None` instead of borrowing the operator's pin, so
    /// [`preflight`] refuses it as unbound rather than comparing a pinned value with itself. The
    /// model binding, provider and endpoint axes are not derivable from artifact bytes, so a caller
    /// that observes them independently supplies them through [`ExoAdmissionPlan::new`].
    #[must_use]
    pub fn identity(&self) -> ExoIdentity {
        ExoIdentity {
            source_revision: EXO_SOURCE_REVISION.to_owned(),
            package_digest: self.package.as_deref().map(sha256_hex),
            extension_digest: self.extension.as_deref().map(sha256_hex),
            bridge_digest: self.bridge.as_deref().map(sha256_hex),
            model_binding: None,
            provider: None,
            endpoint: None,
            prompt_digest: self.prompt.as_deref().map(sha256_hex),
            tool_digest: self.tool.as_deref().map(sha256_hex),
            config_digest: self.config.as_deref().map(sha256_hex),
            contract_version: EXO_CONTRACT_VERSION.to_owned(),
            native_instance_id: None,
        }
    }
}

/// Admission mode selected for the runtime Exo transport seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoAdmissionMode {
    /// Reviewed single-turn admission over the versioned request/decision envelope.
    Enveloped,
    /// Explicit acknowledgement of an un-admitted raw-wire process bridge.
    Legacy,
}

/// Offline admission refusal. Every variant is produced before any transport dispatch.
#[derive(Debug)]
pub enum ExoAdmissionRefusal {
    Descriptor(ExoIdentityError),
    Preflight(ExoPreflightError),
    Admission(ExoAdmissionError),
}

impl std::fmt::Display for ExoAdmissionRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Descriptor(error) => {
                write!(formatter, "reviewed Exo descriptor is invalid: {error}")
            }
            Self::Preflight(error) => write!(formatter, "Exo preflight refused the run: {error}"),
            Self::Admission(error) => write!(formatter, "Exo admission refused the turn: {error}"),
        }
    }
}

impl std::error::Error for ExoAdmissionRefusal {}

impl From<ExoAdmissionRefusal> for String {
    /// Formats a refusal with the production boundary's fail-closed prefix, for callers that
    /// report a startup failure as a message rather than as a typed error.
    fn from(refusal: ExoAdmissionRefusal) -> Self {
        format!("Exo admission refused before any model or game effect: {refusal}")
    }
}

/// One reviewed deployment, admitted offline as exactly one correlated turn.
pub struct ExoAdmissionPlan {
    trusted: ExoTrustedConfiguration,
    inspected: ExoIdentity,
    model_execution_id: String,
    request_id: String,
    turn_id: String,
}

impl ExoAdmissionPlan {
    /// Builds a plan from the operator's pin and the independently inspected deployment identity.
    ///
    /// `inspected` must be the identity observed from the deployment actually on disk (see
    /// [`ExoInspectedArtifacts::identity`]); it is never taken from `trusted`, so the two remain
    /// genuinely independent values for `preflight` to cross-check.
    #[must_use]
    pub fn new(
        trusted: ExoTrustedConfiguration,
        inspected: ExoIdentity,
        model_execution_id: String,
        request_id: String,
        turn_id: String,
    ) -> Self {
        Self {
            trusted,
            inspected,
            model_execution_id,
            request_id,
            turn_id,
        }
    }

    /// Builds a plan whose deployment identity is inspected from the exact artifact bytes, so a
    /// swapped package, extension or bridge fails `preflight` against the pin, and so a pin the
    /// inspection did not bind refuses the run rather than being admitted on the declaration alone.
    #[must_use]
    pub fn inspected(
        trusted: ExoTrustedConfiguration,
        artifacts: &ExoInspectedArtifacts,
        model_execution_id: String,
        request_id: String,
        turn_id: String,
    ) -> Self {
        Self::new(
            trusted,
            artifacts.identity(),
            model_execution_id,
            request_id,
            turn_id,
        )
    }

    /// Builds the reviewed descriptor whose deployment identity axes are the inspected values.
    /// Capability axes stay exactly as shipped by the source review; they are not asserted from, or
    /// on behalf of, the configured bridge.
    fn reviewed_descriptor(&self) -> Result<ExoCapabilityDescriptor, ExoAdmissionRefusal> {
        let mut descriptor =
            ExoCapabilityDescriptor::source_review().map_err(ExoAdmissionRefusal::Descriptor)?;
        descriptor.identity = self.inspected.clone();
        Ok(descriptor)
    }

    /// Runs the offline contract preflight for this deployment without contacting Exo or a model.
    pub fn validate(&self) -> Result<(), ExoAdmissionRefusal> {
        preflight(&self.reviewed_descriptor()?, &self.trusted)
            .map(|_| ())
            .map_err(ExoAdmissionRefusal::Preflight)
    }

    /// Admits one correlated turn over `transport`, or refuses before any dispatch.
    pub fn admit<T: ExoTransport>(
        &self,
        transport: T,
    ) -> Result<ExoAdmittedTransport<T>, ExoAdmissionRefusal> {
        // Refuse before the admitted wrapper exists so a rejected deployment never dispatches.
        self.validate()?;
        let descriptor = self.reviewed_descriptor()?;
        ExoAdmittedTransport::new(
            transport,
            &descriptor,
            &self.trusted,
            self.model_execution_id.clone(),
            self.request_id.clone(),
            self.turn_id.clone(),
        )
        .map_err(ExoAdmissionRefusal::Admission)
    }

    /// Admits the receipt-bound lifecycle adapter. This is the sole capability promotion path:
    /// its concrete type owns a durable lifecycle owner and v2 process effect, whereas the
    /// generic [`Self::admit`] deliberately retains source-review capability states.
    pub fn admit_lifecycle<F: LifecycleManifestFactory>(
        &self,
        transport: ExoLifecycleRuntimeTransport<F>,
    ) -> Result<ExoAdmittedTransport<ExoLifecycleRuntimeTransport<F>>, ExoAdmissionRefusal> {
        let mut descriptor = self.reviewed_descriptor()?;
        descriptor.lifecycle.cancellation = ExoCapabilityState::Supported;
        descriptor.lifecycle.recovery = ExoCapabilityState::Supported;
        descriptor.evidence.turn_identity = ExoCapabilityState::Supported;
        preflight(&descriptor, &self.trusted).map_err(ExoAdmissionRefusal::Preflight)?;
        ExoAdmittedTransport::new(
            transport,
            &descriptor,
            &self.trusted,
            self.model_execution_id.clone(),
            self.request_id.clone(),
            self.turn_id.clone(),
        )
        .map_err(ExoAdmissionRefusal::Admission)
    }
}

/// The transport the runtime hands to its provider after admission.
pub enum AdmittedExoRuntimeTransport<T> {
    Enveloped(Box<ExoAdmittedTransport<T>>),
    Legacy(T),
}

impl<T: ExoTransport> ExoTransport for AdmittedExoRuntimeTransport<T> {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        match self {
            Self::Enveloped(transport) => {
                transport.exchange(request, max_response_bytes, timeout_millis)
            }
            Self::Legacy(transport) => {
                transport.exchange(request, max_response_bytes, timeout_millis)
            }
        }
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        match self {
            Self::Enveloped(transport) => transport.close(),
            Self::Legacy(transport) => transport.close(),
        }
    }
}

/// The reviewed admission decision for one runtime run.
pub enum ExoRuntimeAdmission {
    Enveloped(Box<ExoAdmissionPlan>),
    Legacy,
}

impl ExoRuntimeAdmission {
    /// Admits the reviewed envelope path, or refuses before the run is allowed to continue.
    pub fn enveloped(plan: ExoAdmissionPlan) -> Result<Self, ExoAdmissionRefusal> {
        plan.validate()?;
        Ok(Self::Enveloped(Box::new(plan)))
    }

    /// Records the explicit, un-admitted raw-wire acknowledgement.
    #[must_use]
    pub fn legacy() -> Self {
        Self::Legacy
    }

    #[must_use]
    pub fn mode(&self) -> ExoAdmissionMode {
        match self {
            Self::Enveloped(_) => ExoAdmissionMode::Enveloped,
            Self::Legacy => ExoAdmissionMode::Legacy,
        }
    }

    /// Produces the transport the provider may use, refusing before any dispatch when the
    /// reviewed envelope admission cannot be established.
    pub fn admit<T: ExoTransport>(
        &self,
        transport: T,
    ) -> Result<AdmittedExoRuntimeTransport<T>, ExoAdmissionRefusal> {
        match self {
            Self::Enveloped(plan) => plan
                .admit(transport)
                .map(|admitted| AdmittedExoRuntimeTransport::Enveloped(Box::new(admitted))),
            Self::Legacy => Ok(AdmittedExoRuntimeTransport::Legacy(transport)),
        }
    }
}
