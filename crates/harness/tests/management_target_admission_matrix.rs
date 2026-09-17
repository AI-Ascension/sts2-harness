// SPDX-License-Identifier: MIT

//! Multi-target admission and target-drift matrix for sts2-harness#99 at the
//! management-service boundary: two authorized fake targets, actor-scoped
//! non-disclosure, and every catalog drift code between discovery, preflight
//! and submission. The live adapter's pre-open re-check and restart identity
//! recovery live in `management_target_admission_durability.rs`.
//!
//! Deterministic component evidence over synthetic doubles only. No gateway,
//! provider, native game or host is involved; nothing here proves native
//! behaviour.

#![allow(clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use sts2_harness::management::{
    ManagementClient, ManagementServer, ServerConfig, StaticAuthenticator, TargetAvailability,
    TargetCatalogResponse, TargetDescriptor, decode_strict,
};

#[path = "support/target_catalog_matrix_service.rs"]
mod fixture;
#[path = "support/target_catalog_matrix.rs"]
mod matrix;
#[path = "support/live_workflow.rs"]
mod support;

use fixture::{Outcome, instance_ids, operator, preflight, run_request, two_target_fixture};
use matrix::{
    ALPHA, BETA, CATALOG_REVISION, MatrixCatalog, alpha_descriptor, beta_descriptor, selection,
};

type TargetDrift = fn(&mut TargetDescriptor);

#[test]
fn two_authorized_targets_yield_distinct_exact_admissions_and_hide_unauthorized_targets() -> Outcome
{
    let (catalog, port, service) = two_target_fixture();
    catalog.serve("beta-operator", matrix::catalog(vec![beta_descriptor()]));
    let service = Arc::new(service);
    let full = operator("operator");
    let scoped = operator("beta-operator");

    assert_eq!(instance_ids(&service.target_catalog(&full)?), [ALPHA, BETA]);
    assert_eq!(instance_ids(&service.target_catalog(&scoped)?), [BETA]);

    let authenticator = StaticAuthenticator::new()
        .with_credential("beta-token", scoped.clone())?
        .with_credential("operator-token", full.clone())?;
    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        Arc::new(authenticator),
    )?;
    let server = ManagementServer::start(config, Arc::clone(&service))?;
    let response = ManagementClient::new(server.address(), "beta-token")?.request_json(
        "GET",
        "/v1/workflow-targets",
        None,
    )?;
    assert_eq!(response.status, 200);
    let served: TargetCatalogResponse = decode_strict(&response.body)?;
    assert_eq!(instance_ids(&served), [BETA]);
    assert!(!String::from_utf8_lossy(&response.body).contains(ALPHA));
    server.shutdown()?;

    let alpha = preflight(
        &service,
        &full,
        "request-alpha",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    let beta = preflight(
        &service,
        &full,
        "request-beta",
        selection(&beta_descriptor(), "live.workflow.v2"),
    )?;
    assert_ne!(alpha.target, beta.target);
    assert_ne!(alpha.descriptor_digest, beta.descriptor_digest);
    assert_eq!(alpha.descriptor_digest, alpha_descriptor().digest()?);
    assert_eq!(beta.descriptor_digest, beta_descriptor().digest()?);
    assert_eq!(alpha.catalog_revision, CATALOG_REVISION);
    assert_eq!(beta.catalog_revision, CATALOG_REVISION);
    assert_eq!(beta.target.save_profile.as_deref(), Some("save-beta-v1"));
    assert_eq!(
        beta.target.inference_profile.as_deref(),
        Some("inference-beta-v1")
    );
    assert_eq!(alpha.target.save_profile, None);

    // The scoped actor cannot tell an unauthorized target from a nonexistent one.
    let hidden = preflight(
        &service,
        &scoped,
        "request-hidden",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )
    .expect_err("unauthorized target must not preflight");
    let mut unknown_target = selection(&alpha_descriptor(), "live.workflow.v1");
    unknown_target.instance_id = "instance-gamma".to_owned();
    let unknown = preflight(&service, &scoped, "request-unknown", unknown_target)
        .expect_err("unknown target must not preflight");
    assert_eq!(hidden.code, "target_unavailable");
    assert_eq!(unknown.code, hidden.code);
    assert_eq!(unknown.message, hidden.message);
    let scoped_beta = preflight(
        &service,
        &scoped,
        "request-beta-scoped",
        selection(&beta_descriptor(), "live.workflow.v2"),
    )?;
    assert_eq!(scoped_beta.target, beta.target);
    assert_eq!(scoped_beta.descriptor_digest, beta.descriptor_digest);
    assert_eq!(port.submissions(), 0);

    let alpha_run = service.submit_run(&full, run_request("request-alpha", alpha.clone()))?;
    let beta_run = service.submit_run(&full, run_request("request-beta", beta.clone()))?;
    assert_ne!(alpha_run.workflow_run_id, beta_run.workflow_run_id);
    assert_eq!(port.submissions(), 2);
    let alpha_status = service.status(&full, &alpha_run.workflow_run_id)?.run;
    let beta_status = service.status(&full, &beta_run.workflow_run_id)?.run;
    assert_eq!(alpha_status.admission, Some(alpha));
    assert_eq!(beta_status.admission, Some(beta));
    assert_ne!(alpha_status.admission, beta_status.admission);
    Ok(())
}

#[test]
fn target_removed_between_preflight_and_submit_rejects_before_dispatch() -> Outcome {
    let (catalog, port, service) = two_target_fixture();
    let full = operator("operator");
    let alpha = preflight(
        &service,
        &full,
        "request-removed",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )?;
    catalog.remove_target("operator", ALPHA);

    let error = service
        .submit_run(&full, run_request("request-removed", alpha))
        .expect_err("removed target must not submit");
    assert_eq!(error.code, "target_unavailable");
    assert_eq!(port.submissions(), 0);
    let error = preflight(
        &service,
        &full,
        "request-removed-preflight",
        selection(&alpha_descriptor(), "live.workflow.v1"),
    )
    .expect_err("removed target must not preflight");
    assert_eq!(error.code, "target_unavailable");
    assert_eq!(instance_ids(&service.target_catalog(&full)?), [BETA]);

    // The surviving target is still admitted explicitly; nothing was rerouted.
    let beta = preflight(
        &service,
        &full,
        "request-beta-after-removal",
        selection(&beta_descriptor(), "live.workflow.v1"),
    )?;
    let submitted = service.submit_run(&full, run_request("request-beta-after-removal", beta))?;
    assert_eq!(port.submissions(), 1);
    assert_eq!(
        service
            .status(&full, &submitted.workflow_run_id)?
            .run
            .admission
            .map(|admission| admission.target.instance_id),
        Some(BETA.to_owned())
    );
    Ok(())
}

/// Preflights alpha, applies `drift` to the operator catalog, and asserts the
/// submission and a fresh preflight both reject with `expected` without any
/// execution-port submission.
fn assert_drift_rejects(
    request_id: &str,
    drift: impl FnOnce(&MatrixCatalog),
    expected: &str,
) -> Outcome {
    let (catalog, port, service) = two_target_fixture();
    let full = operator("operator");
    let target = selection(&alpha_descriptor(), "live.workflow.v1");
    let alpha = preflight(&service, &full, request_id, target.clone())?;
    let calls_before = catalog.calls();
    drift(&catalog);

    let error = service
        .submit_run(&full, run_request(request_id, alpha))
        .expect_err("drifted target must not submit");
    assert_eq!(error.code, expected, "submit after drift: {request_id}");
    assert_eq!(port.submissions(), 0, "no effect after drift: {request_id}");
    assert_eq!(
        catalog.calls(),
        calls_before + 1,
        "revalidated at submit: {request_id}"
    );
    if expected == "target_catalog_stale" || expected == "target_descriptor_stale" {
        // Revision drift is recoverable by a fresh preflight, which re-binds.
        let rebound = preflight(&service, &full, request_id, target)?;
        assert_eq!(rebound.request_id, request_id);
        return Ok(());
    }
    let error = preflight(&service, &full, request_id, target)
        .expect_err("drifted target must not preflight");
    assert_eq!(error.code, expected, "preflight after drift: {request_id}");
    Ok(())
}

#[test]
fn revoked_or_expired_target_rejects_before_dispatch_with_stable_codes() -> Outcome {
    let cases: [(&str, TargetAvailability, &str); 3] = [
        (
            "request-revoked",
            TargetAvailability::Revoked,
            "target_revoked",
        ),
        (
            "request-expired",
            TargetAvailability::Expired,
            "target_expired",
        ),
        (
            "request-unavailable",
            TargetAvailability::Unavailable,
            "target_unavailable",
        ),
    ];
    for (request_id, availability, expected) in cases {
        assert_drift_rejects(
            request_id,
            |catalog| {
                catalog.mutate_target("operator", ALPHA, |target| {
                    target.availability = availability;
                });
            },
            expected,
        )?;
    }
    Ok(())
}

#[test]
fn compatibility_or_capability_revision_drift_rejects_before_dispatch() -> Outcome {
    let cases: [(&str, TargetDrift, &str); 2] = [
        (
            "request-compatibility-drift",
            |target| target.compatibility_revision = "live.compatibility.alpha.v2".to_owned(),
            "target_compatibility_stale",
        ),
        (
            "request-capability-drift",
            |target| target.capability_revision = "live.capabilities.alpha.v2".to_owned(),
            "target_capability_stale",
        ),
    ];
    for (request_id, drift, expected) in cases {
        assert_drift_rejects(
            request_id,
            |catalog| catalog.mutate_target("operator", ALPHA, drift),
            expected,
        )?;
    }
    Ok(())
}

#[test]
fn catalog_or_descriptor_revision_drift_rejects_before_dispatch() -> Outcome {
    assert_drift_rejects(
        "request-catalog-drift",
        |catalog| {
            catalog.mutate("operator", |served| {
                served.catalog_revision = "matrix.catalog.v2".to_owned();
            });
        },
        "target_catalog_stale",
    )?;
    // A descriptor change outside the bound selection still invalidates the
    // exact binding, because the binding carries the descriptor digest.
    assert_drift_rejects(
        "request-descriptor-drift",
        |catalog| {
            catalog.mutate_target("operator", ALPHA, |target| {
                target.save_profiles.push("save-alpha-late".to_owned());
            });
        },
        "target_descriptor_stale",
    )
}
