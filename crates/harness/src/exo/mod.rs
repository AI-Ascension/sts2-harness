// SPDX-License-Identifier: MIT

mod client;
mod codex_accounting;
mod contract;
mod decision;
mod protocol;
mod sandbox;
mod session;

pub use client::ExoClient;
pub use codex_accounting::{
    CodexEventAccounting, CodexEventError, CodexStreamStatus, CodexTokenUsage, CodexUsageStatus,
    parse_codex_events,
};
pub use contract::{
    DigestKind, EXO_BRIDGE_WIRE_VERSION, EXO_CAPABILITY_SCHEMA, EXO_CONTRACT_VERSION,
    EXO_DECISION_SCHEMA, EXO_MANIFEST_SCHEMA, EXO_MAP_DECISION_SCHEMA, EXO_MAX_EVENT_BYTES,
    EXO_MAX_RESPONSE_BYTES, EXO_MAX_TURN_TIME_MILLIS, EXO_SOURCE_BASE_REVISION,
    EXO_SOURCE_REVISION, ExoArtifactError, ExoBridgeDecisionEnvelope, ExoBridgeRequestEnvelope,
    ExoBridgeTurn, ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode, ExoControlIdentity,
    ExoDecisionKind, ExoDescriptorError, ExoEvidenceCapabilities, ExoIdentity, ExoIdentityError,
    ExoLifecycleCapabilities, ExoLimits, ExoPlatform, ExoPreflightError, ExoPreflightReport,
    ExoProfile, ExoProfileSupport, ExoRuntime, ExoTerminalOutcome, ExoTrustedConfiguration,
    ExoWireError, ExoWireOutcome, encode_bridge_request, encode_bridge_response,
    exo_bridge_manifest, parse_bridge_decision, parse_bridge_decision_envelope,
    parse_bridge_request, parse_bridge_request_envelope, preflight, responses_capable,
    verify_control_identity, verify_exo_bridge_artifact,
};
pub use decision::{BoundDecision, Decision, DecisionError, parse_decision};
pub use protocol::{
    EXO_MAP_REQUEST_OVERHEAD_BYTES, EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES,
    ExoConfig, ExoDecisionRequest, ExoError, ExoProvider, ExoTransport, ExoTransportError,
};
pub use sandbox::{SandboxError, SanitizedObservation};
pub use session::ExoSession;
