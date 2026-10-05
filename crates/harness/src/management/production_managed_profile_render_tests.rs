// SPDX-License-Identifier: MIT

use super::*;
use crate::management::{
    ExecutionMode, InferenceProfileCatalog, LiveContextRenderPort, LiveInferenceProfileCatalogPort,
    RunTargetConfiguration, TARGET_ADMISSION_SCHEMA_VERSION, TargetAdmissionBinding,
};
use std::sync::{Arc, Mutex};

struct MutableProfileCatalog {
    catalog: Mutex<InferenceProfileCatalog>,
}

impl LiveInferenceProfileCatalogPort for MutableProfileCatalog {
    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<InferenceProfileCatalog, ManagementError> {
        self.catalog
            .lock()
            .map(|catalog| catalog.clone())
            .map_err(|_| ManagementError::store("test_catalog_lock", "catalog lock poisoned"))
    }
}

fn replaced_decision_catalog() -> InferenceProfileCatalog {
    let mut catalog =
        crate::management::synthetic_inference_profile_catalog().expect("synthetic test catalog");
    let descriptor = catalog
        .descriptors
        .iter_mut()
        .find(|descriptor| descriptor.profile_id == "decision.synthetic.v1")
        .expect("synthetic decision profile");
    let mut replacement = descriptor.clone();
    replacement.version = "1.0.1".to_owned();
    *descriptor = replacement.seal().expect("replacement profile digest");
    let catalog = catalog.seal().expect("replacement catalog digest");
    catalog.validate().expect("valid replacement catalog");
    catalog
}

struct ReplacingRenderPort {
    inner: super::RenderPort,
    profiles: Arc<MutableProfileCatalog>,
}

impl LiveContextRenderPort for ReplacingRenderPort {
    fn render_source_for_decision(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
    ) -> Result<ContextRenderSource, ManagementError> {
        let source = self.inner.render_source_for_decision(
            actor,
            request,
            definition_digest,
            binding,
            control_limits,
            input,
            context_ref,
        )?;
        *self.profiles.catalog.lock().expect("profile catalog lock") = replaced_decision_catalog();
        Ok(source)
    }

    fn assert_render_source_current(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
        expected: &ContextRenderSourceIdentity,
    ) -> Result<(), ManagementError> {
        self.inner.assert_render_source_current(
            actor,
            request,
            definition_digest,
            binding,
            control_limits,
            input,
            context_ref,
            expected,
        )
    }

    fn render_required(&self) -> bool {
        self.inner.render_required()
    }
}

#[test]
fn served_managed_render_rechecks_profile_revision_after_preparation_before_provider_write() {
    let initial =
        crate::management::synthetic_inference_profile_catalog().expect("synthetic test catalog");
    let profiles = Arc::new(MutableProfileCatalog {
        catalog: Mutex::new(initial),
    });
    let (source, config) = render_fixture();
    let (mut session, state, exchanges, _) =
        render_test_session(source, config, selected_limits(2), Default::default(), None);
    session.request.admission = Some(TargetAdmissionBinding {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: session.request.request_id.clone(),
        workflow_definition_digest: session.definition_digest.clone(),
        target: RunTargetConfiguration {
            instance_id: session.request.instance_id.clone(),
            execution_profile: session.request.profile.clone(),
            execution_mode: ExecutionMode::Live,
            workflow_revision: "workflow.synthetic.v1".to_owned(),
            compatibility_revision: "compat.synthetic.v1".to_owned(),
            capability_revision: "capability.synthetic.v1".to_owned(),
            game_profile: "synthetic-sts2-v1".to_owned(),
            save_profile: None,
            inference_profile: Some("decision.synthetic.v1".to_owned()),
            context_capability: None,
            provider_capability: None,
        },
        descriptor_digest: "f".repeat(64),
        catalog_revision: "catalog.synthetic.v1".to_owned(),
    });
    let admitted = super::super::super::session::inference_profile::admit_inference_profiles(
        Some(profiles.as_ref()),
        &session.actor,
        &session.request,
        &session.definition,
    )
    .expect("initial profile admission")
    .expect("catalog attached");
    session.inference_profiles = Some(profiles.clone());
    session.admitted_profiles = Some(admitted);
    session.context_render = Some(Arc::new(ReplacingRenderPort {
        inner: super::RenderPort {
            state,
            stale: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            limits: selected_limits(2),
        },
        profiles,
    }));

    let error = session
        .decide_for(&input(), "decision.synthetic.v1", "context.synthetic.v1")
        .expect_err("the profile changed after context preparation");

    assert_eq!(error.code, "inference_profile_binding_changed");
    assert_eq!(exchanges.load(std::sync::atomic::Ordering::SeqCst), 0);
}
