// SPDX-License-Identifier: MIT

mod artifact;
mod descriptor;
mod identity;
mod preflight;
mod strict;
mod wire;
mod wire_types;

pub use artifact::{ExoArtifactError, exo_bridge_manifest, verify_exo_bridge_artifact};
pub use descriptor::{
    ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode, ExoDecisionKind,
    ExoDescriptorError, ExoEvidenceCapabilities, ExoLifecycleCapabilities, ExoLimits, ExoPlatform,
    ExoProfile, ExoProfileSupport, ExoRuntime,
};
pub use identity::{DigestKind, ExoIdentity, ExoIdentityError};
pub use preflight::{
    ExoPreflightError, ExoPreflightReport, ExoTrustedConfiguration, preflight, responses_capable,
    responses_routing_capable,
};
pub use wire::{
    ExoBridgeDecisionEnvelope, ExoBridgeRequestEnvelope, ExoBridgeTurn, ExoControlIdentity,
    ExoTerminalOutcome, ExoWireError, ExoWireOutcome, encode_bridge_request,
    encode_bridge_response, parse_bridge_decision, parse_bridge_decision_envelope,
    parse_bridge_request, parse_bridge_request_envelope, verify_control_identity,
};

/// Immutable contract version implemented by this harness adapter.
pub const EXO_CONTRACT_VERSION: &str = "sts2-exo-bridge-v1";
/// Closed capability descriptor schema.
pub const EXO_CAPABILITY_SCHEMA: &str = "sts2.exo-capability-v1";
/// Frozen source/package manifest schema.
pub const EXO_MANIFEST_SCHEMA: &str = "sts2.exo-manifest-v1";
/// Ordinary decision response schema accepted by `parse_decision`.
pub const EXO_DECISION_SCHEMA: &str = "sts2.exo-decision-v1";
/// Map decision request schema; support remains independently gated.
pub const EXO_MAP_DECISION_SCHEMA: &str = "sts2.exo-decision-map-v1";
/// Envelope version for the supervised request/turn bridge.
pub const EXO_BRIDGE_WIRE_VERSION: &str = "sts2.exo-bridge-wire-v1";
/// Maximum terminal response bytes accepted by the decision parser.
pub const EXO_MAX_RESPONSE_BYTES: usize = 8 * 1024;
/// Maximum event/usage evidence record owned by this contract.
pub const EXO_MAX_EVENT_BYTES: usize = 8 * 1024;
/// Maximum supervised executor turn duration.
pub const EXO_MAX_TURN_TIME_MILLIS: u32 = 120_000;
/// Candidate source revision reviewed by ADR 0017.
pub const EXO_SOURCE_REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";
/// Previous source revision retained only for migration/source-review history.
pub const EXO_SOURCE_BASE_REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";
