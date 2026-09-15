// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used, clippy::panic)]

#[path = "benchmark_manifest/fixtures.rs"]
mod fixtures;

use fixtures::{bytes, document, occurrence, receipt};
use serde_json::{Value, json};
use sts2_harness::benchmark_manifest::{Manifest, ManifestError, TrialStatus};

#[test]
fn binding_preserves_plan_and_manifest_and_is_idempotent_on_exact_receipt() {
    let manifest = Manifest::parse_private(&bytes(&document())).unwrap();
    let original = manifest.export_private().to_vec();
    let plan = manifest.plan_trial(&bytes(&occurrence())).unwrap();
    let bound = plan.bind_seed_receipt(&bytes(&receipt())).unwrap();
    assert_eq!(plan.status(), TrialStatus::Planned);
    assert_eq!(bound.status(), TrialStatus::SeedReceiptBound);
    assert_eq!(bound.manifest().export_private(), original);
    let repeated = bound.bind_seed_receipt(&bytes(&receipt())).unwrap();
    assert_eq!(
        bound.export_private().unwrap(),
        repeated.export_private().unwrap()
    );
    let mut altered = receipt();
    altered["settled"]["correlation_id"] = json!("another-valid-exchange");
    assert_eq!(
        bound.bind_seed_receipt(&bytes(&altered)).unwrap_err(),
        ManifestError::ReceiptConflict
    );
    // Restart can reconstruct a plan and recheck retained bytes; this is not disk/crash evidence.
    let reloaded = Manifest::parse_private(&original)
        .unwrap()
        .plan_trial(&bytes(&occurrence()))
        .unwrap();
    assert_eq!(
        reloaded
            .bind_seed_receipt(&bytes(&receipt()))
            .unwrap()
            .export_private()
            .unwrap(),
        bound.export_private().unwrap()
    );
    assert!(!format!("{bound:?}").contains("ironclad-42"));
}

#[test]
fn distinct_owner_operation_and_fence_substitution_is_rejected() {
    let plan = Manifest::parse_private(&bytes(&document()))
        .unwrap()
        .plan_trial(&bytes(&occurrence()))
        .unwrap();
    for (path, replacement) in [
        ("/plan_digest", json!("b".repeat(64))),
        ("/entry_ordinal", json!(1)),
        ("/settled/instance_id", json!("other-instance")),
        ("/settled/session_id", json!("other-session")),
        ("/settled/lease_id", json!("other-lease")),
        ("/settled/lease_epoch", json!(2)),
        ("/settled/generation", json!(1)),
        ("/settled/operation_id", json!("other-operation")),
        ("/settled/run_mode", json!("diagnostic")),
        ("/operation_id", json!("other-operation")),
    ] {
        let mut candidate = receipt();
        *candidate.pointer_mut(path).unwrap() = replacement;
        assert!(
            plan.bind_seed_receipt(&bytes(&candidate)).is_err(),
            "{path}"
        );
        assert_eq!(plan.status(), TrialStatus::Planned);
    }
    let mut other_occurrence = occurrence();
    other_occurrence["run_id"] = json!("run-2");
    other_occurrence["operation_id"] = json!("op-seed-2");
    let other = plan
        .manifest()
        .plan_trial(&bytes(&other_occurrence))
        .unwrap();
    assert!(other.bind_seed_receipt(&bytes(&receipt())).is_err());
}

#[test]
fn harness_only_occurrence_fields_are_declarations_not_native_attestations() {
    let manifest = Manifest::parse_private(&bytes(&document())).unwrap();
    for path in ["process_id", "run_id", "episode_id", "mcp_session_id"] {
        let mut declared = occurrence();
        declared[path] = json!("other-owner-declaration");
        let plan = manifest.plan_trial(&bytes(&declared)).unwrap();
        // The accepted wire has no such field. Owner must enforce operation/fence uniqueness;
        // this limited receipt-bound state cannot claim detection of illicit tuple reuse.
        assert_eq!(
            plan.bind_seed_receipt(&bytes(&receipt())).unwrap().status(),
            TrialStatus::SeedReceiptBound
        );
    }
}

#[test]
fn requested_and_effective_seed_differ_without_harness_normalization() {
    let mut input = document();
    input["gameplay"]["requested_seed"] = json!("  IrOnClAd-42  ");
    let manifest = Manifest::parse_private(&bytes(&input)).unwrap();
    let plan = manifest.plan_trial(&bytes(&occurrence())).unwrap();
    let mut normalized = receipt();
    normalized["requested_seed"] = input["gameplay"]["requested_seed"].clone();
    normalized["settled"]["requested_seed"] = input["gameplay"]["requested_seed"].clone();
    assert!(plan.bind_seed_receipt(&bytes(&normalized)).is_ok());
    assert_eq!(
        plan.bind_seed_receipt(&bytes(&receipt())).unwrap_err(),
        ManifestError::ReceiptSeedMismatch
    );
    for path in [
        "/settled/canonical_seed",
        "/settled/observation/canonical_seed",
        "/settled/effect_witness/canonical_seed",
    ] {
        *normalized.pointer_mut(path).unwrap() = json!("OTHER");
    }
    assert_eq!(
        plan.bind_seed_receipt(&bytes(&normalized)).unwrap_err(),
        ManifestError::ReceiptSeedMismatch
    );
}

#[test]
fn nonsettlement_malformed_receipts_and_unknown_wrapper_semantics_fail_closed() {
    let plan = Manifest::parse_private(&bytes(&document()))
        .unwrap()
        .plan_trial(&bytes(&occurrence()))
        .unwrap();
    for (path, replacement) in [
        ("/settled/status", json!("accepted")),
        ("/settled/effect_witness", Value::Null),
        ("/settled/observation/generation", json!(0)),
        ("/settled/schema_digest", json!("0".repeat(64))),
        ("/settled/selected_context/ascension", json!(1)),
        ("/settled/observation/host_ready", json!(false)),
        (
            "/settled/observation/phase_after",
            json!("custom_run_setup"),
        ),
        ("/settled/error_code", json!("not-settled")),
        ("/reconcile", json!([null])),
    ] {
        let mut candidate = receipt();
        *candidate.pointer_mut(path).unwrap() = replacement;
        assert!(
            plan.bind_seed_receipt(&bytes(&candidate)).is_err(),
            "{path}"
        );
    }
    for key in [
        "plan_digest",
        "entry_ordinal",
        "requested_seed",
        "operation_id",
        "start",
        "reconcile",
    ] {
        let mut missing = receipt();
        missing.as_object_mut().unwrap().remove(key);
        assert!(plan.bind_seed_receipt(&bytes(&missing)).is_err(), "{key}");
    }
    for path in [
        "",
        "/settled",
        "/settled/observation",
        "/settled/effect_witness",
    ] {
        let mut changed = receipt();
        changed.pointer_mut(path).unwrap()["native_rng_verified"] = json!(true);
        assert!(plan.bind_seed_receipt(&bytes(&changed)).is_err(), "{path}");
    }
    let raw = String::from_utf8(bytes(&receipt()))
        .unwrap()
        .replace("\"lease_id\":", "\"lease_id\":\"foreign\",\"lease_id\":");
    assert!(plan.bind_seed_receipt(raw.as_bytes()).is_err());
    // Recognized raw legacy exchanges remain opaque, bounded private archival bytes.
    let mut archive = receipt();
    archive["start_error"] = json!("synthetic private diagnostic");
    archive["duplicate_start"] = json!({"synthetic_archive":true});
    assert!(plan.bind_seed_receipt(&bytes(&archive)).is_ok());
}

#[test]
fn context_and_nonreceipt_gameplay_facts_have_honest_verification_limits() {
    let mut input = document();
    input["gameplay"]["selected_context"] =
        fixtures::context_variant("\"ascension\":0", "\"ascension\":1");
    let plan = Manifest::parse_private(&bytes(&input))
        .unwrap()
        .plan_trial(&bytes(&occurrence()))
        .unwrap();
    assert_eq!(
        plan.bind_seed_receipt(&bytes(&receipt())).unwrap_err(),
        ManifestError::ReceiptContextMismatch
    );
    let mut unavailable = document();
    unavailable["gameplay"]["platform"]["runtime_version"] = json!("different-runtime");
    let manifest = Manifest::parse_private(&bytes(&unavailable)).unwrap();
    assert!(
        !manifest
            .compare(&Manifest::parse_private(&bytes(&document())).unwrap())
            .is_empty()
    );
    // Neither the schema nor receipt attests this declaration; it must not yield native verification.
    assert_eq!(
        manifest
            .plan_trial(&bytes(&occurrence()))
            .unwrap()
            .bind_seed_receipt(&bytes(&receipt()))
            .unwrap()
            .status(),
        TrialStatus::SeedReceiptBound
    );
}

#[test]
fn occurrence_parser_rejects_missing_fields_duplicates_and_invalid_fences() {
    let manifest = Manifest::parse_private(&bytes(&document())).unwrap();
    for key in occurrence().as_object().unwrap().keys() {
        let mut missing = occurrence();
        missing.as_object_mut().unwrap().remove(key);
        assert!(manifest.plan_trial(&bytes(&missing)).is_err(), "{key}");
    }
    for (key, replacement) in [
        ("lease_epoch", json!(9007199254740992_u64)),
        ("request_generation", json!(-1)),
        ("entry_ordinal", json!(1024)),
        ("process_id", json!("x".repeat(129))),
        ("run_mode", json!("unsupported")),
        ("plan_digest", json!("bad")),
    ] {
        let mut invalid = occurrence();
        invalid[key] = replacement;
        assert!(manifest.plan_trial(&bytes(&invalid)).is_err(), "{key}");
    }
    let mut same_text = occurrence();
    same_text["mcp_session_id"] = same_text["gateway_session_id"].clone();
    let plan = manifest.plan_trial(&bytes(&same_text)).unwrap();
    assert!(plan.bind_seed_receipt(&bytes(&receipt())).is_ok());
}
