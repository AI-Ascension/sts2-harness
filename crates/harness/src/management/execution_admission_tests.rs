// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use crate::management::{
    MANAGEMENT_SCHEMA_VERSION, RunTargetConfiguration, TARGET_ADMISSION_SCHEMA_VERSION,
    digest_value,
};

fn matching_admission() -> (RunRequest, String) {
    let definition: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("definition fixture");
    let digest = digest_value(&definition).expect("definition digest");
    let admission = TargetAdmissionBinding {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: "request-exec-admission".to_owned(),
        workflow_definition_digest: digest.clone(),
        target: RunTargetConfiguration {
            instance_id: "instance-1".to_owned(),
            execution_profile: "live.workflow.v1".to_owned(),
            execution_mode: ExecutionMode::Live,
            workflow_revision: "0.1.0".to_owned(),
            compatibility_revision: "live.compatibility.v1".to_owned(),
            capability_revision: "live.capabilities.v1".to_owned(),
            game_profile: "sts2-live-v1".to_owned(),
            save_profile: None,
            inference_profile: None,
            context_capability: None,
            provider_capability: None,
        },
        descriptor_digest: "0".repeat(64),
        catalog_revision: "live.catalog.v1".to_owned(),
    };
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "request-exec-admission".to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: Some(admission),
    };
    (request, digest)
}

#[test]
fn mismatched_admission_is_rejected_at_the_execution_boundary() {
    type Tamper = fn(&mut TargetAdmissionBinding);
    let cases: [(&str, Tamper, &str); 4] = [
        (
            "instance",
            |admission| admission.target.instance_id = "instance-2".to_owned(),
            "target_instance_mismatch",
        ),
        (
            "stale_revision",
            |admission| admission.target.workflow_revision = "9.9.9".to_owned(),
            "target_admission_stale",
        ),
        (
            "definition_digest",
            |admission| admission.workflow_definition_digest = "0".repeat(64),
            "target_admission_digest_mismatch",
        ),
        (
            "request_identity",
            |admission| admission.request_id = "other-request".to_owned(),
            "target_request_mismatch",
        ),
    ];
    for (label, tamper, expected) in cases {
        let (request, digest) = matching_admission();
        let mut admission = request.admission.clone().expect("admission");
        tamper(&mut admission);
        let error = validate_live_admission(&request, &digest, &admission)
            .expect_err("mismatched admission must be rejected");
        assert_eq!(error.code, expected, "case {label}");
    }
}
