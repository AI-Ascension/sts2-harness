// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::context_memory::*;

fn scope() -> MemoryScope {
    MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
}

fn over_limit_policy() -> MemoryPolicy {
    MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "policy-migration-bounds".to_owned(),
        version: 1,
        scope: scope(),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("revision-1".to_owned()),
        corpus_generation: 1,
        rolling_same_episode_sources: false,
        cross_scope: false,
        approved_summary_catalog: Vec::new(),
        ranker_version: "lexical-v1".to_owned(),
        query_derivation_version: "derive-v1".to_owned(),
        max_candidates: MAX_CANDIDATES,
        max_results: MAX_RESULTS,
        max_selected: MAX_SELECTED,
        optional_byte_budget: MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    }
}

#[test]
fn migration_records_bound_source_bytes_and_violation_shape() {
    let corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    let capabilities = corpus.capabilities();
    let policy = over_limit_policy();
    let bytes = serde_json::to_vec(&policy).expect("policy bytes");

    let mut oversized = vec![b' '; MEMORY_POLICY_MIGRATION_MAX_BYTES + 1];
    oversized.extend_from_slice(&bytes);
    assert_eq!(
        PolicyMigrationProposal::new_from_bytes(&oversized, &capabilities, "migration-bounds"),
        Err(MemoryError::InvalidProposal)
    );

    let mut duplicate =
        PolicyMigrationProposal::new_from_bytes(&bytes, &capabilities, "migration-bounds")
            .expect("proposal");
    duplicate.violations.push(duplicate.violations[0].clone());
    assert_eq!(duplicate.validate(), Err(MemoryError::InvalidProposal));

    duplicate
        .violations
        .truncate(MEMORY_POLICY_MIGRATION_MAX_VIOLATIONS);
    duplicate.state = PolicyMigrationState::Adopted;
    duplicate.adopted_policy_sha256 = None;
    assert_eq!(duplicate.validate(), Err(MemoryError::InvalidProposal));
}
