// SPDX-License-Identifier: MIT

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::management::{SeedModeV2, SeedRequestV2, WORKFLOW_SEED_REQUEST_V2_SCHEMA};

use super::{PlanDocument, PlanEntry, ReservationRecord, SeedTransportConfig};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const PROBE_MODE: &str = "STS2_SEED_ADMISSION_TEST_PROBE";
const PROBE_NAME: &str = "runtime_support::seed_transport::admission_tests::environment_probe";

fn authored_seed(seed: &str) -> SeedRequestV2 {
    SeedRequestV2 {
        schema_version: WORKFLOW_SEED_REQUEST_V2_SCHEMA.to_owned(),
        mode: SeedModeV2::Explicit,
        seed: Some(seed.to_owned()),
    }
}

#[test]
fn new_seed_admission_matches_authored_utf8_and_whitespace_rules() -> TestResult {
    for seed in [
        String::new(),
        String::from(" padded "),
        String::from("\u{a0}padded"),
        String::from("padded\u{2003}"),
        String::from("\u{2003}"),
        String::from("control\nseed"),
        String::from("control\u{85}seed"),
        "a".repeat(65),
        format!("{}a", "é".repeat(32)),
        format!("{}a", "🦀".repeat(16)),
    ] {
        assert!(authored_seed(&seed).validate().is_err());
        run_probe(&seed, false, "refused")?;
    }
    for seed in [
        String::from("seed-42"),
        String::from("interior space"),
        String::from("interior\u{a0}space"),
        "a".repeat(64),
        "é".repeat(32),
        "🦀".repeat(16),
    ] {
        authored_seed(&seed).validate()?;
        run_probe(&seed, false, "new")?;
    }
    Ok(())
}

#[test]
fn legacy_padded_seed_recovers_exactly_and_never_creates_a_new_reservation() -> TestResult {
    for seed in [" padded ", "\u{a0}padded", "padded\u{2003}"] {
        run_probe(seed, true, "recovery")?;
    }
    Ok(())
}

#[test]
fn legacy_seed_requires_the_original_reservation_identity_at_admission() -> TestResult {
    run_probe(" padded ", true, "conflict")
}

#[test]
fn noncanonical_unselected_entries_allow_only_exact_plan_recovery() -> TestResult {
    for (seeds, ordinal) in [
        (["canonical", " padded "], 0),
        ([" padded ", "canonical"], 1),
    ] {
        run_plan_probe(&seeds, ordinal, false, "refused")?;
        run_plan_probe(&seeds, ordinal, true, "recovery")?;
    }
    Ok(())
}

fn run_probe(seed: &str, persisted: bool, mode: &str) -> TestResult {
    run_plan_probe(&[seed], 0, persisted, mode)
}

fn run_plan_probe(seeds: &[&str], ordinal: u64, persisted: bool, mode: &str) -> TestResult {
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "sts2-seed-admission-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir(&directory)?;
    let path = directory.join("reservation.json");
    let plan = PlanDocument {
        entries: seeds
            .iter()
            .enumerate()
            .map(|(index, seed)| PlanEntry {
                ordinal: index as u64,
                requested_seed: (*seed).to_owned(),
            })
            .collect(),
    };
    let seed = *seeds
        .get(ordinal as usize)
        .ok_or("fixture ordinal is absent")?;
    let mut config = SeedTransportConfig::fixture_for_tests(seed, "seed-admission-test");
    config.plan_digest = super::validation::sha256_json(&plan)?;
    config.entry_ordinal = ordinal;
    if persisted {
        let record = ReservationRecord {
            protocol: String::from("seeded-run-reservation-v1"),
            operation_id: if mode == "conflict" {
                String::from("different-operation")
            } else {
                config.operation_id.clone()
            },
            plan_digest: config.plan_digest.clone(),
            entry_ordinal: config.entry_ordinal,
            requested_seed: seed.to_owned(),
            run_mode: config.run_mode.clone(),
            context_digest: config.context_digest().to_owned(),
            selected_context: config.selected_context(),
            generation: 7,
            status: String::from("unknown"),
            settled_receipt: None,
        };
        std::fs::write(&path, serde_json::to_vec(&record)?)?;
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", PROBE_NAME, "--nocapture"])
        .env(PROBE_MODE, mode)
        .env("STS2_SEED_PLAN_JSON", serde_json::to_string(&plan)?)
        .env("STS2_SEED_ENTRY_ORDINAL", ordinal.to_string())
        .env("STS2_SEED_OPERATION_ID", &config.operation_id)
        .env("STS2_SEED_RUN_MODE", &config.run_mode)
        .env(
            "STS2_SEED_CONTEXT_JSON",
            config.selected_context().to_string(),
        )
        .env("STS2_SEED_RESERVATION_PATH", &path)
        .env("STS2_SEED_VERIFY_IDEMPOTENCY", "false");
    for name in ["STS2_SEED_PLAN_DIGEST", "STS2_SEED_CONTEXT_DIGEST"] {
        command.env_remove(name);
    }
    let output = command.output()?;
    std::fs::remove_dir_all(&directory)?;
    assert!(
        output.status.success(),
        "seed admission probe {mode} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn environment_probe() -> TestResult {
    let Ok(mode) = std::env::var(PROBE_MODE) else {
        return Ok(());
    };
    let path = std::path::PathBuf::from(std::env::var("STS2_SEED_RESERVATION_PATH")?);
    let before = std::fs::read(&path).ok();
    let result = SeedTransportConfig::from_environment();
    if mode == "refused" || mode == "conflict" {
        assert!(
            result.is_err(),
            "invalid new seed or foreign reservation admitted"
        );
        assert_eq!(std::fs::read(&path).ok(), before);
        return Ok(());
    }
    let config = result?.ok_or("configured seed plan is absent")?;
    let plan: PlanDocument = serde_json::from_str(&std::env::var("STS2_SEED_PLAN_JSON")?)?;
    let selected = plan
        .entries
        .iter()
        .find(|entry| entry.ordinal == config.entry_ordinal())
        .ok_or("selected fixture entry is absent")?;
    assert_eq!(config.requested_seed(), selected.requested_seed);
    let arguments = config.start_arguments("instance", "session", "lease", 1, 99);
    assert_eq!(arguments["seed"], config.requested_seed());
    let reservation = config.reserve_at(path.clone(), 99)?;
    if mode == "new" {
        assert!(!reservation.resumed);
        assert_eq!(reservation.request_generation(), 99);
    } else {
        assert_eq!(mode, "recovery");
        assert!(reservation.resumed);
        assert_eq!(reservation.request_generation(), 7);
        assert_eq!(std::fs::read(&path).ok(), before);
        let mut changed: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        changed["operation_id"] = "different-operation".into();
        std::fs::write(&path, serde_json::to_vec(&changed)?)?;
        assert!(config.reserve_at(path.clone(), 99).is_err());
        std::fs::remove_file(&path)?;
        assert!(config.reserve_at(path.clone(), 99).is_err());
        assert!(
            !path.exists(),
            "legacy recovery created a fresh reservation"
        );
    }
    Ok(())
}
