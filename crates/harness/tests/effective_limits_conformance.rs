// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use serde_json::Value;
use sts2_harness::context_memory::*;
use sts2_harness::effective_limits::*;
use sts2_harness::provider_session::*;

fn memory_scope() -> MemoryScope {
    MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
}

fn session_scope() -> SessionScope {
    SessionScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope")
}

fn corpus() -> MemoryCorpus {
    MemoryCorpus::with_limits(memory_scope(), 16, 4096).expect("corpus")
}

fn memory_policy(
    candidates: usize,
    results: usize,
    selected: usize,
    optional: usize,
) -> MemoryPolicy {
    MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "policy-limits".to_owned(),
        version: 1,
        scope: memory_scope(),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("revision-1".to_owned()),
        corpus_generation: 1,
        rolling_same_episode_sources: false,
        cross_scope: false,
        approved_summary_catalog: Vec::new(),
        ranker_version: "lexical-v1".to_owned(),
        query_derivation_version: "derive-v1".to_owned(),
        max_candidates: candidates,
        max_results: results,
        max_selected: selected,
        optional_byte_budget: optional,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    }
}

fn session_policy(turns: usize, ttl: u64) -> ProviderSessionPolicy {
    let mut policy = ProviderSessionPolicy::disabled(session_scope());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    policy.max_completed_turns = turns;
    policy.history_ttl_seconds = ttl;
    policy
}

fn limit_maximum(schema: &Value, field: &str) -> Option<u64> {
    schema["properties"]["effective_limits"]["properties"][field]["maximum"].as_u64()
}

fn property_maximum(schema: &Value, field: &str) -> Option<u64> {
    schema["properties"][field]["maximum"].as_u64()
}

/// The published descriptor field name may differ from the portable policy field name.
fn policy_field(field: &str) -> &str {
    match field {
        "max_history_ttl_seconds" => "history_ttl_seconds",
        other => other,
    }
}

#[test]
fn records_publish_every_value_and_match_the_closed_schema() {
    let record = corpus().capabilities().effective_limit_record();
    assert_eq!(record.surface, "context-memory");
    record.validate().expect("memory record");
    let optional = record.row("optional_byte_budget").expect("optional budget");
    assert_eq!(optional.class, LimitClass::SchemaBroaderThanExecutable);
    assert_eq!(optional.policy_schema_ceiling, Some(65_536));
    assert_eq!(optional.executable_ceiling, 8_192);
    assert_eq!(record.executable_ceiling("max_entries_per_run"), Some(16));

    let session = NativeCapabilities::fixture().effective_limit_record();
    assert_eq!(session.surface, "provider-session");
    session.validate().expect("session record");
    let turns = session.row("max_completed_turns").expect("completed turns");
    assert_eq!(turns.policy_schema_ceiling, Some(1_024));
    assert_eq!(turns.executable_ceiling, 128);
    let ttl = session.row("max_history_ttl_seconds").expect("ttl");
    assert_eq!(ttl.policy_schema_ceiling, Some(604_800));
    assert_eq!(ttl.executable_ceiling, 86_400);

    let schema: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/effective-limits.schema.json"
    ))
    .expect("record schema");
    let validator = jsonschema::validator_for(&schema).expect("record validator");
    for published in [&record, &session] {
        let value = serde_json::to_value(published).expect("record value");
        assert!(validator.is_valid(&value), "{value}");
    }
    let capability_schema: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/provider-session/capabilities.schema.json"
    ))
    .expect("session capabilities schema");
    let validator = jsonschema::validator_for(&capability_schema).expect("capabilities validator");
    let value = serde_json::to_value(NativeCapabilities::fixture()).expect("descriptor");
    assert!(validator.is_valid(&value), "{value}");
}

#[test]
fn published_ceilings_agree_with_policy_capability_and_runtime_schemas() {
    let memory_record = corpus().capabilities().effective_limit_record();
    let session_record = NativeCapabilities::fixture().effective_limit_record();
    let cases = [
        (
            &memory_record,
            "context-memory",
            include_bytes!("../../../contracts/context-memory/policy.schema.json").as_slice(),
            include_bytes!("../../../contracts/context-memory/capabilities.schema.json").as_slice(),
        ),
        (
            &session_record,
            "provider-session",
            include_bytes!("../../../contracts/provider-session/policy.schema.json").as_slice(),
            include_bytes!("../../../contracts/provider-session/capabilities.schema.json")
                .as_slice(),
        ),
    ];
    for (record, surface, policy_bytes, capability_bytes) in cases {
        let policy: Value = serde_json::from_slice(policy_bytes).expect("policy schema");
        let capability: Value =
            serde_json::from_slice(capability_bytes).expect("capability schema");
        assert_eq!(record.surface, surface);
        for row in &record.rows {
            assert_eq!(
                Some(row.capabilities_schema_ceiling),
                limit_maximum(&capability, &row.field),
                "{surface}: {} capability ceiling",
                row.field
            );
            assert_eq!(
                row.policy_schema_ceiling,
                property_maximum(&policy, policy_field(&row.field)),
                "{surface}: {} policy ceiling",
                row.field
            );
            assert!(row.capabilities_schema_ceiling >= row.executable_ceiling);
        }
    }
}

#[test]
fn reduced_profile_keeps_the_capability_schema_ceiling() {
    let mut capabilities = corpus().capabilities();
    capabilities.effective_limits.max_source_bytes = 1_024;
    capabilities.effective_limits.max_sources_per_job = 1;
    let record = capabilities.effective_limit_record();
    record.validate().expect("reduced record");
    let capability: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/context-memory/capabilities.schema.json"
    ))
    .expect("capability schema");
    for (field, executable) in [
        ("max_source_bytes", 1_024_u64),
        ("max_sources_per_job", 1_u64),
    ] {
        let row = record.row(field).expect("row");
        assert_eq!(row.executable_ceiling, executable);
        assert_eq!(
            Some(row.capabilities_schema_ceiling),
            limit_maximum(&capability, field),
            "{field} keeps the capability schema ceiling"
        );
        assert!(row.capabilities_schema_ceiling > row.executable_ceiling);
        assert_eq!(record.admit(field, executable), Ok(()));
        assert_eq!(
            record.admit(field, executable + 1),
            Err(UnavailableReason::EffectiveLimitExceeded)
        );
    }
}

#[test]
fn every_limit_admits_lower_and_exact_and_rejects_one_over() {
    let records = [
        corpus().capabilities().effective_limit_record(),
        NativeCapabilities::fixture().effective_limit_record(),
    ];
    for record in records {
        for row in &record.rows {
            assert_eq!(record.admit(&row.field, 1), Ok(()), "{}", row.field);
            assert_eq!(
                record.admit(&row.field, row.executable_ceiling),
                Ok(()),
                "{}",
                row.field
            );
            assert_eq!(
                record.admit(&row.field, row.executable_ceiling.saturating_add(1)),
                Err(UnavailableReason::EffectiveLimitExceeded),
                "{}",
                row.field
            );
        }
        assert_eq!(
            record.admit("no-such-field", 1),
            Err(UnavailableReason::FieldNotAdvertised)
        );
    }
}

#[test]
fn schema_validity_and_profile_admission_are_separate_outcomes() {
    let corpus = corpus();
    let capabilities = corpus.capabilities();
    for (candidates, results, selected, optional) in [
        (64, 16, 32, 8_192),
        (64, 16, 32, 65_536),
        (64, 16, 32, 65_537),
    ] {
        let policy = memory_policy(candidates, results, selected, optional);
        let schema_valid = policy.validate_schema().is_ok();
        assert_eq!(schema_valid, optional <= 65_536);
        assert_eq!(
            capabilities.admit_policy_value("optional_byte_budget", optional as u64),
            if optional <= 8_192 {
                Ok(())
            } else {
                Err(UnavailableReason::EffectiveLimitExceeded)
            }
        );
    }
    assert_eq!(
        capabilities.admit_policy_value("optional_byte_budget", 65_536),
        Err(UnavailableReason::EffectiveLimitExceeded)
    );
    let over_limit = memory_policy(64, 16, 32, 65_536);
    assert_eq!(
        over_limit.validate_against_capabilities(&corpus, &capabilities),
        Err(MemoryError::CapabilityLimitExceeded {
            limit: "optional_byte_budget".to_owned(),
            requested: 65_536,
            effective: 8_192,
        })
    );

    let session = NativeCapabilities::fixture();
    for (turns, ttl) in [(128_usize, 86_400_u64), (1_024, 604_800), (1_025, 604_801)] {
        let policy = session_policy(turns, ttl);
        assert_eq!(
            policy.validate_schema().is_ok(),
            turns <= 1_024 && ttl <= 604_800
        );
        assert_eq!(
            session.admit_policy_value("max_completed_turns", turns as u64),
            if turns <= 128 {
                Ok(())
            } else {
                Err(UnavailableReason::EffectiveLimitExceeded)
            }
        );
        assert_eq!(
            session.admit_policy_value("max_history_ttl_seconds", ttl),
            if ttl <= 86_400 {
                Ok(())
            } else {
                Err(UnavailableReason::EffectiveLimitExceeded)
            }
        );
    }
}

#[test]
fn disabled_surface_reports_disabled_rather_than_unlimited() {
    let mut corpus = corpus();
    corpus.set_enabled(false);
    let record = corpus.capabilities().effective_limit_record();
    assert!(!record.enabled);
    assert_eq!(
        record.admit("max_candidates", 1),
        Err(UnavailableReason::Disabled)
    );
    assert_eq!(
        record.admit("max_candidates", 64),
        Err(UnavailableReason::Disabled)
    );
}
