// SPDX-License-Identifier: MIT

// Hand-authored MIT synthetic suite fixtures. No game files, real profiles, native seeds,
// provider payloads or private identifiers.
use sts2_harness::benchmark_manifest::suite::{
    NativeWitness, PlannedSuiteTrial, PolicyConfig, SUITE_VERSION, SeedCase, SeedCorpus,
    SuiteBudgets, SuiteManifest, plan,
};

/// Every metric label a synthetic suite predeclares.
pub const METRICS: [&str; 5] = [
    "reached_floor",
    "action_count",
    "latency_millis",
    "provider_tokens",
    "cost_micros",
];

/// Builds the 2-seed x 2-policy x 2-repetition synthetic suite.
pub fn manifest() -> SuiteManifest {
    SuiteManifest {
        version: SUITE_VERSION.to_owned(),
        benchmark_ref: "benchmark:synthetic-2x2x2".to_owned(),
        corpus: SeedCorpus {
            cases: vec![
                SeedCase {
                    case_id: "case-alpha".to_owned(),
                    native_game_seed: 987_654_321,
                },
                SeedCase {
                    case_id: "case-beta".to_owned(),
                    native_game_seed: 1_234_567_890,
                },
            ],
            corpus_randomization_seed: 444_000_222,
            provider_sampling_seed: 555_000_111,
        },
        policies: vec![
            PolicyConfig {
                policy_id: "policy-exo".to_owned(),
                settings_digest: "a".repeat(64),
            },
            PolicyConfig {
                policy_id: "policy-local".to_owned(),
                settings_digest: "b".repeat(64),
            },
        ],
        repetitions: 2,
        evaluator_revision: "evaluator-2026-09-23".to_owned(),
        budgets: SuiteBudgets {
            max_steps_per_trial: 500,
            max_total_steps: 4_000,
            max_total_duration_millis: 600_000,
            max_concurrency: 4,
            max_provider_spend_micros: Some(1_000_000),
        },
        metrics: METRICS.iter().map(|metric| (*metric).to_owned()).collect(),
    }
}

/// Returns the deterministic plan of the synthetic suite.
pub fn planned() -> Vec<PlannedSuiteTrial> {
    plan(&manifest()).unwrap()
}

/// Returns the stable trial key for one (case, policy, repetition) cell.
pub fn key_of(case_id: &str, policy_id: &str, repetition: u32) -> String {
    planned()
        .into_iter()
        .find(|trial| {
            trial.case_id == case_id
                && trial.policy_id == policy_id
                && trial.repetition == repetition
        })
        .unwrap()
        .trial_key
}

/// Builds a verified initial witness from one repeated byte.
pub fn witness(byte: u8) -> NativeWitness {
    NativeWitness::parse(&format!(
        "asc-state:v1:sha256:{}",
        format!("{byte:02x}").repeat(32)
    ))
    .unwrap()
}
