// SPDX-License-Identifier: MIT

//! Production-boundary tests for the runtime Exo admission gate.
//!
//! These tests drive the same [`ExoAdmissionPlan`] the runtime hands to its provider seam. A
//! refused deployment must not dispatch the inner transport at all, so it cannot produce a model
//! call or a game effect.

#![allow(clippy::expect_used)]

use std::{cell::RefCell, rc::Rc};

use sts2_harness::exo_admission::{
    ExoAdmissionPlan, ExoAdmissionRefusal, ExoInspectedArtifacts, ExoRuntimeAdmission,
};
use sts2_harness::{
    EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoAdmittedTransport, ExoCapabilityDescriptor,
    ExoCapabilityState, ExoContextMode, ExoIdentity, ExoLimits, ExoPlatform, ExoPreflightError,
    ExoProfile, ExoRestrictedProfile, ExoRuntime, ExoTransport, ExoTransportError,
    ExoTrustedConfiguration, preflight,
};

#[derive(Default)]
struct Recording {
    exchanges: usize,
    closes: usize,
}

struct Transport {
    recording: Rc<RefCell<Recording>>,
}

impl ExoTransport for Transport {
    fn exchange(
        &mut self,
        _request: &[u8],
        _maximum: usize,
        _timeout: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.recording.borrow_mut().exchanges += 1;
        Ok(vec![b' '; 2])
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.recording.borrow_mut().closes += 1;
        Ok(())
    }
}

fn recording_transport(recording: &Rc<RefCell<Recording>>) -> Transport {
    Transport {
        recording: recording.clone(),
    }
}

fn complete_identity() -> ExoIdentity {
    ExoIdentity {
        source_revision: EXO_SOURCE_REVISION.to_owned(),
        package_digest: Some(String::from("a").repeat(64)),
        extension_digest: Some(String::from("b").repeat(64)),
        bridge_digest: Some(String::from("c").repeat(64)),
        model_binding: Some(String::from("gpt-5-pro")),
        provider: Some(String::from("openai")),
        endpoint: Some(String::from("https://api.openai.com/v1")),
        prompt_digest: Some(String::from("d").repeat(64)),
        tool_digest: Some(String::from("e").repeat(64)),
        config_digest: Some(String::from("f").repeat(64)),
        contract_version: EXO_CONTRACT_VERSION.to_owned(),
        native_instance_id: Some(String::from("native-1")),
    }
}

fn trusted(identity: ExoIdentity) -> ExoTrustedConfiguration {
    ExoTrustedConfiguration {
        identity,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: ExoRestrictedProfile::reviewed_private("/var/lib/sts2-harness/exo-admission"),
    }
}

fn plan(trusted: ExoTrustedConfiguration) -> ExoAdmissionPlan {
    // These plans describe a deployment whose advertised identity is its operator pin, so the tests
    // below exercise the capability, profile, route, schema and revision gates. The identity binding
    // itself is covered by the inspection tests, which derive the advertised identity from bytes.
    let inspected = trusted.identity.clone();
    ExoAdmissionPlan::new(
        trusted,
        inspected,
        String::from("execution-1"),
        String::from("request-1"),
        String::from("turn-1"),
    )
}

fn bound_artifacts() -> ExoInspectedArtifacts {
    ExoInspectedArtifacts {
        package: Some(b"package bytes".to_vec()),
        extension: Some(b"extension bytes".to_vec()),
        bridge: Some(b"bridge bytes".to_vec()),
        prompt: Some(b"prompt bytes".to_vec()),
        tool: Some(b"tool bytes".to_vec()),
        config: Some(b"config bytes".to_vec()),
    }
}

fn inspected_plan(
    configured: ExoTrustedConfiguration,
    artifacts: &ExoInspectedArtifacts,
) -> ExoAdmissionPlan {
    ExoAdmissionPlan::inspected(
        configured,
        artifacts,
        String::from("execution-1"),
        String::from("request-1"),
        String::from("turn-1"),
    )
}

#[test]
fn reviewed_envelope_refuses_an_unverified_minimum_capability_without_dispatching() {
    let plan = plan(trusted(complete_identity()));
    let refusal = plan
        .validate()
        .expect_err("unverified capability must refuse");
    assert!(matches!(
        refusal,
        ExoAdmissionRefusal::Preflight(ExoPreflightError::RequiredCapability(
            "evidence.turn_identity"
        ))
    ));
    assert!(refusal.to_string().contains("preflight"));

    let recording = Rc::new(RefCell::new(Recording::default()));
    let result = plan.admit(recording_transport(&recording));
    assert!(result.is_err());
    assert_eq!(recording.borrow().exchanges, 0);
    assert_eq!(recording.borrow().closes, 0);
}

fn refusal_of(trusted: ExoTrustedConfiguration) -> ExoAdmissionRefusal {
    plan(trusted)
        .validate()
        .expect_err("hostile deployment must refuse")
}

#[test]
fn reviewed_envelope_refuses_unreviewed_or_unsupported_deployments_before_dispatch() {
    let mut unreviewed = complete_identity();
    unreviewed.source_revision = String::from("a").repeat(40);
    assert!(matches!(
        refusal_of(trusted(unreviewed)),
        ExoAdmissionRefusal::Preflight(ExoPreflightError::UnreviewedSourceRevision)
    ));

    let mut incomplete = complete_identity();
    incomplete.package_digest = None;
    assert!(matches!(
        refusal_of(trusted(incomplete)),
        ExoAdmissionRefusal::Preflight(ExoPreflightError::MissingIdentity)
    ));

    let mut malformed = complete_identity();
    malformed.bridge_digest = Some(String::from("z").repeat(64));
    assert!(matches!(
        refusal_of(trusted(malformed)),
        ExoAdmissionRefusal::Preflight(ExoPreflightError::InvalidDescriptor(_))
    ));

    let mut routed = complete_identity();
    routed.endpoint = Some(String::from("https://openrouter.ai/api/v1"));
    assert!(matches!(
        refusal_of(trusted(routed)),
        ExoAdmissionRefusal::Preflight(ExoPreflightError::RoutingNotResponsesCapable)
    ));

    let mut unsupported_runtime = trusted(complete_identity());
    unsupported_runtime.runtime = ExoRuntime::ChatCompletions;
    assert!(matches!(
        refusal_of(unsupported_runtime),
        ExoAdmissionRefusal::Preflight(ExoPreflightError::RuntimeUnsupported)
    ));

    let mut unsupported_profile = trusted(complete_identity());
    unsupported_profile.profile = ExoProfile::Map;
    assert!(matches!(
        refusal_of(unsupported_profile),
        ExoAdmissionRefusal::Preflight(ExoPreflightError::RequiredCapability(_))
    ));
}

#[test]
fn envelope_constructor_refuses_before_a_provider_or_transport_exists() {
    let mut unsupported_profile = trusted(complete_identity());
    unsupported_profile.profile = ExoProfile::Expert;
    let plan = plan(unsupported_profile);
    assert!(ExoRuntimeAdmission::enveloped(plan).is_err());
}

#[test]
fn legacy_acknowledgement_dispatches_raw_bytes_and_is_not_an_admitted_boundary() {
    let recording = Rc::new(RefCell::new(Recording::default()));
    let admission = ExoRuntimeAdmission::legacy();
    let mut transport = admission
        .admit(recording_transport(&recording))
        .expect("legacy acknowledgement");
    assert!(transport.exchange(b"{}", 16, 10).is_ok());
    assert_eq!(recording.borrow().exchanges, 1);
    transport.close().expect("close");
    assert_eq!(recording.borrow().closes, 1);
}

#[test]
fn admitted_turn_reports_zero_model_calls_before_it_dispatches() {
    let mut descriptor = ExoCapabilityDescriptor::source_review().expect("descriptor");
    descriptor.identity = complete_identity();
    descriptor.evidence.turn_identity = ExoCapabilityState::Supported;
    descriptor.lifecycle.cancellation = ExoCapabilityState::Supported;
    descriptor.lifecycle.recovery = ExoCapabilityState::Supported;
    let configured = trusted(complete_identity());
    let recording = Rc::new(RefCell::new(Recording::default()));
    let admitted = ExoAdmittedTransport::new(
        recording_transport(&recording),
        &descriptor,
        &configured,
        String::from("execution-1"),
        String::from("request-1"),
        String::from("turn-1"),
    )
    .expect("synthetic admission");
    assert_eq!(admitted.admission().model_calls, 0);
    assert_eq!(recording.borrow().exchanges, 0);
}

/// The inspected (advertised) deployment identity must be cross-checked against the operator pin
/// before the capability gate, so a swapped artifact is refused as an identity mismatch instead of
/// being masked by an unrelated capability refusal.
#[test]
fn swapped_artifact_bytes_are_refused_as_an_identity_mismatch_before_the_capability_gate() {
    let mut descriptor = ExoCapabilityDescriptor::source_review().expect("source descriptor");
    descriptor.identity = complete_identity();
    let mut configured = trusted(complete_identity());
    configured.identity.package_digest = Some(String::from("9").repeat(64));
    assert_eq!(
        preflight(&descriptor, &configured),
        Err(ExoPreflightError::IdentityMismatch("package_digest")),
        "a swapped package artifact must fail identity binding"
    );
}

/// The advertised identity is inspected from real bytes, so a bridge whose bytes differ from the
/// pin is refused as an identity mismatch before the capability gate can mask it, and no transport
/// is ever dispatched.
#[test]
fn inspected_bridge_bytes_that_do_not_match_the_pin_refuse_before_any_dispatch() {
    let artifacts = bound_artifacts();
    let mut configured = trusted(complete_identity());
    configured.identity.package_digest = artifacts.package.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.extension_digest =
        artifacts.extension.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.prompt_digest = artifacts.prompt.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.tool_digest = artifacts.tool.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.config_digest = artifacts.config.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.bridge_digest = Some(String::from("3").repeat(64));
    let plan = inspected_plan(configured, &artifacts);

    let refusal = plan.validate().expect_err("a swapped bridge must refuse");
    assert!(
        matches!(
            refusal,
            ExoAdmissionRefusal::Preflight(ExoPreflightError::IdentityMismatch("bridge_digest"))
        ),
        "the inspected bridge bytes must be compared against the pin, got {refusal}"
    );
    let recording = Rc::new(RefCell::new(Recording::default()));
    assert!(plan.admit(recording_transport(&recording)).is_err());
    assert_eq!(recording.borrow().exchanges, 0);
    assert_eq!(recording.borrow().closes, 0);
}

/// Binding one axis to inspected bytes does not relax the others: with the package pair matching the
/// pin, the next axis no artifact backs is still refused as unbound, so the binding cannot be widened
/// into blanket admission.
#[test]
fn a_matching_package_binding_leaves_every_other_unbound_axis_refused() {
    let artifacts = ExoInspectedArtifacts {
        package: Some(b"package bytes".to_vec()),
        bridge: Some(b"bridge bytes".to_vec()),
        ..ExoInspectedArtifacts::default()
    };
    let mut configured = trusted(complete_identity());
    configured.identity.package_digest = Some(sts2_harness::sha256_hex(b"package bytes"));
    configured.identity.bridge_digest = Some(sts2_harness::sha256_hex(b"bridge bytes"));
    let plan = inspected_plan(configured, &artifacts);

    let refusal = plan.validate().expect_err("an unbound axis must refuse");
    assert!(
        matches!(
            refusal,
            ExoAdmissionRefusal::Preflight(ExoPreflightError::UnboundIdentity("extension_digest"))
        ),
        "the matched package must not admit the unbound extension axis, got {refusal}"
    );
    let recording = Rc::new(RefCell::new(Recording::default()));
    assert!(plan.admit(recording_transport(&recording)).is_err());
    assert_eq!(recording.borrow().exchanges, 0);
    assert_eq!(recording.borrow().closes, 0);
}

/// A pinned axis that the inspection did not bind cannot be admitted from the operator's declaration
/// alone: preflight refuses it as unbound, so an uninspected artifact never reaches a model or game
/// effect.
#[test]
fn a_pinned_axis_without_inspected_bytes_refuses_before_any_dispatch() {
    let artifacts = ExoInspectedArtifacts {
        bridge: Some(b"bridge bytes".to_vec()),
        ..ExoInspectedArtifacts::default()
    };
    let plan = inspected_plan(trusted(complete_identity()), &artifacts);

    let refusal = plan.validate().expect_err("an unbound pin must refuse");
    assert!(
        matches!(
            refusal,
            ExoAdmissionRefusal::Preflight(ExoPreflightError::UnboundIdentity("package_digest"))
        ),
        "a pin without inspected bytes must fail closed, got {refusal}"
    );
    let recording = Rc::new(RefCell::new(Recording::default()));
    assert!(plan.admit(recording_transport(&recording)).is_err());
    assert_eq!(recording.borrow().exchanges, 0);
    assert_eq!(recording.borrow().closes, 0);
}

/// The inspected identity is the hash of the inspected bytes, never a copy of the pin, so a plan
/// built from bytes the operator did not pin is refused even when every other axis is pinned.
#[test]
fn inspected_package_bytes_that_do_not_match_the_pin_refuse_before_any_dispatch() {
    let artifacts = bound_artifacts();
    let mut configured = trusted(complete_identity());
    configured.identity.package_digest = Some(String::from("9").repeat(64));
    configured.identity.extension_digest =
        artifacts.extension.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.bridge_digest = artifacts.bridge.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.prompt_digest = artifacts.prompt.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.tool_digest = artifacts.tool.as_deref().map(sts2_harness::sha256_hex);
    configured.identity.config_digest = artifacts.config.as_deref().map(sts2_harness::sha256_hex);
    let plan = inspected_plan(configured, &artifacts);

    let refusal = plan.validate().expect_err("a swapped package must refuse");
    assert!(
        matches!(
            refusal,
            ExoAdmissionRefusal::Preflight(ExoPreflightError::IdentityMismatch("package_digest"))
        ),
        "swapped package bytes must fail identity binding, got {refusal}"
    );
    let recording = Rc::new(RefCell::new(Recording::default()));
    assert!(plan.admit(recording_transport(&recording)).is_err());
    assert_eq!(recording.borrow().exchanges, 0);
    assert_eq!(recording.borrow().closes, 0);
}
