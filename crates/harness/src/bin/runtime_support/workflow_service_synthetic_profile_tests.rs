// SPDX-License-Identifier: MIT

use super::super::super::super::{
    runtime_v3_admission::RuntimeV3AdmissionMode, runtime_v3_settings::live_admission,
};
use super::super::LIVE_CONTEXT_REF;
use super::*;
use sts2_harness::management::{
    INFERENCE_PROFILE_SCHEMA_VERSION, InferenceProfileBudgets, InferenceProfileContinuity,
    InferenceProfileDescriptor, InferenceProfileGrants, InferenceProfileState,
};

struct ClosedSource;

impl sts2_harness::DecisionSource for ClosedSource {
    fn decide(
        &mut self,
        _input: &sts2_harness::DecisionInput,
    ) -> Result<sts2_harness::Decision, sts2_harness::PolicyError> {
        Err(sts2_harness::PolicyError::InputBlocked)
    }
}

fn admitted_profile() -> Result<AdmittedInferenceProfileBinding, String> {
    let descriptor = InferenceProfileDescriptor {
        schema_version: INFERENCE_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: LIVE_DECISION_PROFILE_ID.to_owned(),
        version: "1.0.0".to_owned(),
        digest: String::new(),
        adapter: LIVE_DECISION_ADAPTER.to_owned(),
        requested_model: "reviewed-model".to_owned(),
        resolved_model: None,
        prompt_revision: "a".repeat(64),
        settings_revision: "b".repeat(64),
        supported_settings: vec!["max_provider_calls".to_owned()],
        operations: vec!["decide".to_owned()],
        node_kinds: vec!["decide".to_owned()],
        context_compatibility: vec![LIVE_CONTEXT_REF.to_owned()],
        continuity: InferenceProfileContinuity {
            provider_session_continuity: false,
            survives_controller_restart: false,
        },
        effective_budgets: InferenceProfileBudgets {
            max_input_bytes: 131_072,
            max_output_tokens: 2_000_000,
            max_provider_calls: 10_000,
        },
        grants: InferenceProfileGrants {
            select: true,
            edit: false,
        },
        state: InferenceProfileState::Available,
    }
    .seal()
    .map_err(|error| error.to_string())?;
    let profile_ref = format!(
        "{}:{}:{}",
        descriptor.profile_id, descriptor.version, descriptor.digest
    );
    Ok(AdmittedInferenceProfileBinding {
        binding: sts2_harness::management::InferenceProfileBinding {
            graph_id: "main".to_owned(),
            node_id: "decision-node".to_owned(),
            node_kind: "decide".to_owned(),
            profile_ref,
            profile_id: descriptor.profile_id.clone(),
            version: descriptor.version.clone(),
            digest: descriptor.digest.clone(),
            adapter: descriptor.adapter.clone(),
            requested_model: descriptor.requested_model.clone(),
            resolved_model: descriptor.resolved_model.clone(),
        },
        descriptor,
    })
}

#[test]
fn served_synthetic_profile_keeps_exact_decision_and_context_live_fences() -> Result<(), String> {
    let profile = admitted_profile()?;
    let profile_ref = profile.binding.profile_ref.clone();
    assert_eq!(profile.binding.profile_id, "decision.live.v1");
    let source = ProfileBoundDecisionSource {
        source: Box::new(ClosedSource),
        profiles: vec![profile],
    };

    assert!(source.accepts(&profile_ref, "context.live.v1"));
    assert!(!source.accepts(&profile_ref, "context.synthetic.v1"));
    assert!(!source.accepts("decision.synthetic.v1:1.0.0:unbound", "context.live.v1"));
    assert!(!source.accepts("decision.live.v2:1.0.0:unbound", "context.live.v1"));

    let mut changed_node = admitted_profile()?;
    changed_node.binding.node_kind = "observe".to_owned();
    let changed_source = ProfileBoundDecisionSource {
        source: Box::new(ClosedSource),
        profiles: vec![changed_node],
    };
    assert!(!changed_source.accepts(&profile_ref, "context.live.v1"));
    Ok(())
}

#[test]
fn synthetic_envelope_refuses_live_episode_admission() {
    assert!(matches!(
        live_admission::resolve(None, true, RuntimeV3AdmissionMode::SyntheticEnvelope),
        Err(error) if error.contains("synthetic-envelope") && error.contains("live episodes")
    ));
}
