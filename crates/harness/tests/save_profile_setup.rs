// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Admission matrix for the capability-gated save-profile setup mapping
//! (issue #102, T1). Synthetic only: no gateway is called and no profile is
//! created, selected or read.

use sts2_harness::management::{
    PROFILE_ROUTE_CONTRACT, PROFILE_ROUTE_REVISION, PROFILE_SETUP_SCHEMA_VERSION,
    ProfileBaselineFence, ProfileReadback, ProfileSetupError, ProfileSetupGrants,
    ProfileSetupOperation, ProfileSetupOperationDocument, ProfileSetupRequest, admit_profile_setup,
    is_profile_identity,
};

const INSTANCE: &str = "instance-1";
const PROFILE: &str = "save.live.v1";

fn digest(seed: u8) -> String {
    std::iter::repeat_n(format!("{seed:02x}"), 32).collect()
}

fn all_grants() -> ProfileSetupGrants {
    ProfileSetupGrants {
        discovery: true,
        selection: true,
        provisioning: true,
    }
}

fn request(document: ProfileSetupOperationDocument) -> ProfileSetupRequest {
    ProfileSetupRequest {
        schema_version: PROFILE_SETUP_SCHEMA_VERSION.to_owned(),
        instance_id: INSTANCE.to_owned(),
        operation: document,
        profile_id: None,
        baseline: None,
        operation_id: None,
        run_active: false,
    }
}

#[test]
fn each_operation_maps_to_one_fixed_tool_and_route() {
    let cases = [
        (
            ProfileSetupOperation::List,
            "sts2.save_profile_list",
            "/v1/instances/instance-1/save-profiles",
        ),
        (
            ProfileSetupOperation::Current,
            "sts2.save_profile_current",
            "/v1/instances/instance-1/save-profile/current",
        ),
        (
            ProfileSetupOperation::Select,
            "sts2.save_profile_select",
            "/v1/instances/instance-1/save-profile/select",
        ),
        (
            ProfileSetupOperation::CreateDisposable,
            "sts2.save_profile_create_disposable",
            "/v1/instances/instance-1/save-profile/create-disposable",
        ),
    ];
    for (operation, tool, route) in cases {
        assert_eq!(operation.tool(), tool);
        assert_eq!(operation.route_path(INSTANCE, None), route);
    }
    assert_eq!(
        ProfileSetupOperation::Status.route_path(INSTANCE, Some("op-1")),
        "/v1/instances/instance-1/save-profile/operations/op-1"
    );
}

#[test]
fn read_and_mutation_operations_are_separated() {
    assert!(!ProfileSetupOperation::List.is_mutation());
    assert!(!ProfileSetupOperation::Current.is_mutation());
    assert!(!ProfileSetupOperation::Status.is_mutation());
    assert!(ProfileSetupOperation::Select.is_mutation());
    assert!(ProfileSetupOperation::CreateDisposable.is_mutation());
    assert_eq!(
        PROFILE_ROUTE_REVISION, "save-profile-v1-mcp",
        "the accepted MCP revision is pinned"
    );
    assert_eq!(PROFILE_ROUTE_CONTRACT, "gateway-save-profile-v1");
}

#[test]
fn listing_admits_under_discovery_only() {
    let grants = ProfileSetupGrants {
        discovery: true,
        selection: false,
        provisioning: false,
    };
    let admitted = admit_profile_setup(&request(ProfileSetupOperationDocument::List), grants)
        .expect("discovery grant admits a list");
    assert_eq!(admitted.tool(), "sts2.save_profile_list");
    assert!(!admitted.is_mutation());
}

#[test]
fn ungranted_operations_are_refused_before_any_effect() {
    assert_eq!(
        admit_profile_setup(
            &request(ProfileSetupOperationDocument::List),
            ProfileSetupGrants::none()
        ),
        Err(ProfileSetupError::PermissionDenied)
    );
    let mut select = request(ProfileSetupOperationDocument::Select);
    select.profile_id = Some(PROFILE.to_owned());
    select.baseline = Some(ProfileBaselineFence {
        profile_id: PROFILE.to_owned(),
        baseline_digest: digest(0x11),
    });
    // Discovery-only deployment cannot select.
    let discovery_only = ProfileSetupGrants {
        discovery: true,
        ..ProfileSetupGrants::none()
    };
    assert_eq!(
        admit_profile_setup(&select, discovery_only),
        Err(ProfileSetupError::PermissionDenied)
    );
}

#[test]
fn discovery_cannot_name_a_profile_or_a_baseline() {
    let mut disguised = request(ProfileSetupOperationDocument::List);
    disguised.profile_id = Some(PROFILE.to_owned());
    assert_eq!(
        admit_profile_setup(&disguised, all_grants()),
        Err(ProfileSetupError::DiscoveryMustBeEffectFree)
    );

    let mut fenced = request(ProfileSetupOperationDocument::Current);
    fenced.baseline = Some(ProfileBaselineFence {
        profile_id: PROFILE.to_owned(),
        baseline_digest: digest(0x22),
    });
    assert_eq!(
        admit_profile_setup(&fenced, all_grants()),
        Err(ProfileSetupError::DiscoveryMustBeEffectFree)
    );
}

#[test]
fn selection_requires_a_matching_baseline_fence() {
    let mut select = request(ProfileSetupOperationDocument::Select);
    select.profile_id = Some(PROFILE.to_owned());
    assert_eq!(
        admit_profile_setup(&select, all_grants()),
        Err(ProfileSetupError::BaselineFenceMismatch)
    );

    select.baseline = Some(ProfileBaselineFence {
        profile_id: "save.other.v1".to_owned(),
        baseline_digest: digest(0x33),
    });
    assert_eq!(
        admit_profile_setup(&select, all_grants()),
        Err(ProfileSetupError::BaselineFenceMismatch)
    );

    select.baseline = Some(ProfileBaselineFence {
        profile_id: PROFILE.to_owned(),
        baseline_digest: digest(0x44),
    });
    let admitted = admit_profile_setup(&select, all_grants()).expect("fenced selection");
    assert!(admitted.is_mutation());
    assert_eq!(admitted.profile_id(), Some(PROFILE));
}

#[test]
fn selection_without_a_profile_identity_is_refused() {
    let mut select = request(ProfileSetupOperationDocument::Select);
    select.baseline = Some(ProfileBaselineFence {
        profile_id: PROFILE.to_owned(),
        baseline_digest: digest(0x55),
    });
    assert_eq!(
        admit_profile_setup(&select, all_grants()),
        Err(ProfileSetupError::ProfileRequired)
    );
}

#[test]
fn a_disposable_provision_may_not_fence_a_baseline_it_does_not_have() {
    let mut provision = request(ProfileSetupOperationDocument::CreateDisposable);
    provision.baseline = Some(ProfileBaselineFence {
        profile_id: PROFILE.to_owned(),
        baseline_digest: digest(0x66),
    });
    assert_eq!(
        admit_profile_setup(&provision, all_grants()),
        Err(ProfileSetupError::BaselineFenceMismatch)
    );

    let admitted = admit_profile_setup(&provision_unfenced(), all_grants()).expect("provision");
    assert!(admitted.is_mutation());
    assert_eq!(admitted.profile_id(), None);
}

fn provision_unfenced() -> ProfileSetupRequest {
    request(ProfileSetupOperationDocument::CreateDisposable)
}

#[test]
fn an_active_run_refuses_a_mutation_but_not_discovery() {
    let mut provision = request(ProfileSetupOperationDocument::CreateDisposable);
    provision.run_active = true;
    assert_eq!(
        admit_profile_setup(&provision, all_grants()),
        Err(ProfileSetupError::ActiveRunConflict)
    );

    let mut list = request(ProfileSetupOperationDocument::List);
    list.run_active = true;
    admit_profile_setup(&list, all_grants()).expect("discovery is admitted while a run is active");
}

#[test]
fn unsupported_schema_and_identity_shapes_are_refused() {
    let mut future = request(ProfileSetupOperationDocument::List);
    future.schema_version = "ascension.save-profile-setup/v2".to_owned();
    assert_eq!(
        admit_profile_setup(&future, all_grants()),
        Err(ProfileSetupError::Incompatible)
    );

    let mut traversal = request(ProfileSetupOperationDocument::Status);
    traversal.operation_id = Some("../../etc/passwd".to_owned());
    assert_eq!(
        admit_profile_setup(&traversal, all_grants()),
        Err(ProfileSetupError::InvalidRequest)
    );

    let mut url = request(ProfileSetupOperationDocument::List);
    url.instance_id = "https://example.invalid".to_owned();
    assert_eq!(
        admit_profile_setup(&url, all_grants()),
        Err(ProfileSetupError::InvalidRequest)
    );
}

#[test]
fn receipt_reconciliation_requires_an_operation_identity() {
    let status = request(ProfileSetupOperationDocument::Status);
    assert_eq!(
        admit_profile_setup(&status, all_grants()),
        Err(ProfileSetupError::InvalidRequest)
    );
    let mut named = status;
    named.operation_id = Some("op-7".to_owned());
    let admitted = admit_profile_setup(&named, all_grants()).expect("status");
    assert_eq!(
        admitted.route_path(),
        "/v1/instances/instance-1/save-profile/operations/op-7"
    );
}

#[test]
fn readback_must_match_the_admitted_identity_before_setup_progresses() {
    let mut select = request(ProfileSetupOperationDocument::Select);
    select.profile_id = Some(PROFILE.to_owned());
    let baseline = digest(0x77);
    select.baseline = Some(ProfileBaselineFence {
        profile_id: PROFILE.to_owned(),
        baseline_digest: baseline.clone(),
    });
    let admitted = admit_profile_setup(&select, all_grants()).expect("selection");

    let wrong_identity = ProfileReadback {
        profile_id: "save.other.v1".to_owned(),
        baseline_digest: baseline.clone(),
        available: true,
    };
    assert_eq!(
        wrong_identity.verify(&admitted, Some(&baseline)),
        Err(ProfileSetupError::ReadbackMismatch)
    );

    let unavailable = ProfileReadback {
        profile_id: PROFILE.to_owned(),
        baseline_digest: baseline.clone(),
        available: false,
    };
    assert_eq!(
        unavailable.verify(&admitted, Some(&baseline)),
        Err(ProfileSetupError::ReadbackMismatch)
    );

    let drifted = ProfileReadback {
        profile_id: PROFILE.to_owned(),
        baseline_digest: digest(0x88),
        available: true,
    };
    assert_eq!(
        drifted.verify(&admitted, Some(&baseline)),
        Err(ProfileSetupError::ReadbackMismatch)
    );

    let matching = ProfileReadback {
        profile_id: PROFILE.to_owned(),
        baseline_digest: baseline.clone(),
        available: true,
    };
    let verified = matching
        .verify(&admitted, Some(&baseline))
        .expect("matching readback");
    assert_eq!(verified.profile_id(), PROFILE);
}

#[test]
fn discovery_readback_cannot_be_used_as_a_mutation_readback() {
    // A list mapping names no profile, so no readback can verify against it:
    // discovery must not be able to advance a downstream setup step.
    let admitted = admit_profile_setup(&request(ProfileSetupOperationDocument::List), all_grants())
        .expect("list");
    let readback = ProfileReadback {
        profile_id: PROFILE.to_owned(),
        baseline_digest: digest(0x99),
        available: true,
    };
    assert_eq!(
        readback.verify(&admitted, None),
        Err(ProfileSetupError::ReadbackMismatch)
    );
}

#[test]
fn malformed_baseline_digests_are_refused_at_admission_and_readback() {
    let mut select = request(ProfileSetupOperationDocument::Select);
    select.profile_id = Some(PROFILE.to_owned());
    select.baseline = Some(ProfileBaselineFence {
        profile_id: PROFILE.to_owned(),
        baseline_digest: "NOT-A-DIGEST".to_owned(),
    });
    assert_eq!(
        admit_profile_setup(&select, all_grants()),
        Err(ProfileSetupError::InvalidRequest)
    );
}

#[test]
fn request_deserialization_rejects_unknown_members() {
    let unknown = r#"{"schema_version":"ascension.save-profile-setup/v1","instance_id":"instance-1",
        "operation":"list","profile_id":null,"baseline":null,"operation_id":null,
        "run_active":false,"host_path":"/tmp/x"}"#;
    assert!(serde_json::from_str::<ProfileSetupRequest>(unknown).is_err());
}

#[test]
fn portable_identities_refuse_paths_urls_and_oversized_values() {
    assert!(is_profile_identity("save.live.v1"));
    assert!(!is_profile_identity("../escape"));
    assert!(!is_profile_identity("https://example.invalid/p"));
    assert!(!is_profile_identity("C:\\saves\\run"));
    assert!(!is_profile_identity(""));
    assert!(!is_profile_identity(&"a".repeat(129)));
}
