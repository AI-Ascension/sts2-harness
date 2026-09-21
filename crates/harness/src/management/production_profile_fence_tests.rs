// SPDX-License-Identifier: MIT

//! Served-live inference-profile dispatch fence.
//!
//! `#94` requires that an invalid binding produces *no* effect and no synthetic
//! success.  The admitted run keeps the exact revision it resolved at admission,
//! so an owner that replaced that revision -- or a caller that names a profile the
//! run never admitted -- is refused before the provider exchange instead of being
//! silently re-bound to another revision.  A run that re-bound here would spend a
//! provider call on a revision nobody admitted, which is exactly the failure these
//! two cases pin.
//!
//! The composition under test is the real production session assembled by
//! [`ProductionLiveWorkflowSessionFactory`]; only the gateway/MCP runtime, the
//! provider and the owner-served profile catalog are fixtures.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use super::{Served, observation};
use crate::management::synthetic_inference_profile_catalog;
use std::sync::{Arc, Mutex};

/// The synthetic owner's catalog with its decision profile served at `version`,
/// re-sealed and validated the way a genuine owner replacement would be.
fn catalog_at_decision_version(version: &str) -> InferenceProfileCatalog {
    let mut catalog = synthetic_inference_profile_catalog().expect("synthetic catalog");
    for descriptor in &mut catalog.descriptors {
        if descriptor.profile_id == "decision.synthetic.v1" {
            let mut replaced = descriptor.clone();
            replaced.version = version.to_owned();
            *descriptor = replaced.seal().expect("seal replaced descriptor");
        }
    }
    let catalog = catalog.seal().expect("seal replaced catalog");
    catalog.validate().expect("validate replaced catalog");
    catalog
}

/// An owner-served catalog a test can replace between admission and dispatch.
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

fn open_serving(profiles: Arc<MutableProfileCatalog>) -> (Served, Box<dyn LiveWorkflowSession>) {
    let served = Served::admitted();
    let factory = served.factory_with_profiles(observation("combat-1", 1), profiles);
    let session = served.open(&factory).expect("admitted session");
    (served, session)
}

#[test]
fn served_decide_for_refuses_a_profile_revision_replaced_after_admission() {
    let profiles = Arc::new(MutableProfileCatalog {
        catalog: Mutex::new(catalog_at_decision_version("1.0.0")),
    });
    let (served, mut session) = open_serving(Arc::clone(&profiles));
    session.launch().expect("session launch");
    // The owner replaces the admitted decision revision before the decision.
    *profiles.catalog.lock().unwrap() = catalog_at_decision_version("1.0.1");

    let input = served.decision_input(observation("combat-1", 1));
    let error = session
        .decide_for(&input, "decision.synthetic.v1", "context.synthetic.v1")
        .expect_err("replaced revision");
    assert_eq!(error.code, "inference_profile_binding_changed");
    let (_, _, decide_calls, _) = served.counts();
    assert_eq!(
        decide_calls, 0,
        "a replaced revision must not be re-bound or paid for at the provider"
    );
}

#[test]
fn served_decide_for_refuses_a_profile_outside_the_admitted_bindings() {
    let profiles = Arc::new(MutableProfileCatalog {
        catalog: Mutex::new(catalog_at_decision_version("1.0.0")),
    });
    let (served, mut session) = open_serving(profiles);
    session.launch().expect("session launch");

    let input = served.decision_input(observation("combat-1", 1));
    let error = session
        .decide_for(&input, "decision.unadmitted.v1", "context.synthetic.v1")
        .expect_err("unadmitted profile");
    assert_eq!(error.code, "inference_profile_binding_unknown");
    let (_, _, decide_calls, _) = served.counts();
    assert_eq!(
        decide_calls, 0,
        "an unadmitted profile must not reach the provider"
    );
}
