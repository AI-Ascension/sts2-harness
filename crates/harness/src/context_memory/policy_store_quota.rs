// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::context_memory::{policy_owner::SavedPolicy, *};

fn saved(version: u64, padding: bool) -> SavedPolicy {
    let policy = MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "quota-policy".to_owned(),
        version,
        scope: MemoryScope::new("project", "run", "episode", "agent"),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("revision".to_owned()),
        corpus_generation: 1,
        rolling_same_episode_sources: false,
        cross_scope: false,
        approved_summary_catalog: Vec::new(),
        ranker_version: "lexical-v1".to_owned(),
        query_derivation_version: "derive-v1".to_owned(),
        max_candidates: 64,
        max_results: 16,
        max_selected: 32,
        optional_byte_budget: 9000,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    };
    let mut raw = serde_json::to_vec(&policy).unwrap();
    if padding {
        raw.resize(MAX_POLICY_BYTES, b' ');
    }
    SavedPolicy::new(&raw, &policy.scope).unwrap()
}

#[test]
fn aggregate_serialized_quota_rolls_back_staged_valid_records() {
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/policy-quota")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("policies.sqlite");
    let mut store = PolicyStore::open(
        &path,
        [7; 32],
        MemoryScope::new("project", "run", "episode", "agent"),
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    let first = saved(1, false);
    let reference = first.reference.clone();
    store
        .change(|journal| journal.insert_policy(first.clone()), || Ok(()))
        .unwrap();
    // Stage one bounded transaction rather than repeatedly re-encrypting progressively larger
    // histories. Every policy is individually valid and the count is exactly at its own cap.
    let result = store.change(
        |journal| {
            for version in 2..=MAX_POLICY_VERSIONS as u64 {
                journal.insert_policy(saved(version, true))?;
            }
            Ok(())
        },
        || Ok(()),
    );
    assert_eq!(result, Err(PolicyOwnerError::Capacity));
    let retained = store.load().unwrap();
    assert_eq!(retained.policies.len(), 1);
    assert_eq!(
        retained.policy(&reference).unwrap().raw_bytes(),
        first.raw_bytes()
    );
    assert!(std::fs::metadata(&path).unwrap().len() <= MAX_POLICY_DATABASE_BYTES);
    drop(store);
    std::fs::remove_dir_all(directory).unwrap();
}
