// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used, clippy::panic)]

#[path = "benchmark_manifest/fixtures.rs"]
mod fixtures;

use fixtures::{bytes, context_variant, document, occurrence};
use serde_json::{Value, json};
use sts2_harness::benchmark_manifest::{MAX_MANIFEST_BYTES, Manifest, ManifestError, Mismatch};

fn parse(value: &Value) -> Manifest {
    Manifest::parse_private(&bytes(value)).unwrap()
}

#[test]
fn canonical_roundtrip_and_member_order_have_stable_domain_separated_identity() {
    let input = document();
    let manifest = parse(&input);
    assert_eq!(
        manifest.configuration_digest_private(),
        "871d1751d9aef2ccc2a21629b7612b524af664892dcc7a9324259ff234943c76"
    );
    assert_eq!(
        manifest.experiment_digest_private(),
        "cc1aff908b8d82c98c5338494b17a438a28a68e03d2e304686d932be4468f164"
    );
    assert_eq!(
        manifest.artifact_digest_private(),
        "b245f9f8f10c39851e12b38990077b1da4fab276289ade6cd6fa6f594d5371c8"
    );
    assert_eq!(
        input["gameplay"]["selected_context"]["context_digest"],
        "d57563180f198b73970510427981504a9df10c62931577e601f9dcce6275fbe9"
    );
    let again = Manifest::parse_private(manifest.export_private()).unwrap();
    let pretty = Manifest::parse_private(&serde_json::to_vec_pretty(&input).unwrap()).unwrap();
    assert_eq!(manifest.export_private(), again.export_private());
    assert_eq!(manifest.export_private(), pretty.export_private());
    assert_eq!(
        manifest.configuration_digest_private(),
        again.configuration_digest_private()
    );
    assert_ne!(
        manifest.configuration_digest_private(),
        manifest.experiment_digest_private()
    );
    assert_ne!(
        manifest.experiment_digest_private(),
        manifest.artifact_digest_private()
    );
    // Canonical field ordering differs from the lexically sorted source Value.
    assert!(
        manifest
            .export_private()
            .starts_with(br#"{"version":"ascension.benchmark-manifest.v1""#)
    );
    assert!(manifest.compare(&again).is_empty());
}

#[test]
fn every_noncontext_gameplay_leaf_changes_configuration_identity() {
    let original = document();
    let baseline = parse(&original);
    let mut leaves = Vec::new();
    string_leaves(&original["gameplay"], "/gameplay", &mut leaves);
    for path in leaves {
        if path.starts_with("/gameplay/selected_context/")
            || path == "/gameplay/profile_artifact/baseline_digest"
        {
            continue;
        }
        let mut changed = original.clone();
        let old = changed.pointer(&path).unwrap().as_str().unwrap();
        let replacement = if matches!(old.len(), 40 | 64) {
            "c".repeat(old.len())
        } else {
            format!("{old}-other")
        };
        *changed.pointer_mut(&path).unwrap() = replacement.into();
        let candidate = parse(&changed);
        assert_ne!(
            baseline.configuration_digest_private(),
            candidate.configuration_digest_private(),
            "{path}"
        );
        assert!(!baseline.compare(&candidate).is_empty(), "{path}");
    }
}

fn string_leaves(value: &Value, prefix: &str, out: &mut Vec<String>) {
    if value.is_string() {
        out.push(prefix.to_owned());
    }
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            string_leaves(value, &format!("{prefix}/{key}"), out);
        }
    }
}

#[test]
fn context_profile_build_act_order_and_modifier_changes_are_bound() {
    let original = document();
    let baseline = parse(&original);
    for (from, to) in [
        (
            "standard/ironclad/asc0/fresh",
            "standard/ironclad/asc1/fresh",
        ),
        ("\"ascension\":0", "\"ascension\":1"),
        ("\"modifiers\":[]", "\"modifiers\":[\"synthetic-modifier\"]"),
        ("\"act_1\",\"act_2\"", "\"act_2\",\"act_1\""),
        ("standard_default", "explicit_selection"),
        ("fresh-standard-comparison", "other-baseline"),
        (
            "4581aaf95348126550cdf3b73ec46b39d447523cf7cb35aec71c2842d1945031",
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ),
        (
            "\"save_policy\":\"disabled\"",
            "\"save_policy\":\"enabled\"",
        ),
        ("sts2-game/v0.107.1", "sts2-game/v0.107.2"),
        (
            "2db9d9f665c776c2324c7f98134b900a8b3332f32148ea1cb84063d52db94ff4",
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ),
        ("ai-ascension/sts2-game-mod", "synthetic-mod"),
        (
            "0c6e7bbb54996222a4894de999fc860decc360f9936c9bd5a7382d97b361bb7e",
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ),
    ] {
        let mut changed = original.clone();
        let context = context_variant(from, to);
        changed["gameplay"]["profile_artifact"]["baseline_digest"] =
            context["profile_baseline"]["digest"].clone();
        changed["gameplay"]["selected_context"] = context;
        let candidate = parse(&changed);
        assert_ne!(
            baseline.configuration_digest_private(),
            candidate.configuration_digest_private(),
            "{from}"
        );
        assert!(
            baseline
                .compare(&candidate)
                .contains(&Mismatch::SelectedContext)
        );
    }
    for (from, to) in [
        ("\"character\":\"ironclad\"", "\"character\":\"other\""),
        ("\"game_mode\":\"standard\"", "\"game_mode\":\"other\""),
        ("\"kind\":\"fresh\"", "\"kind\":\"existing\""),
        ("\"act_1\",\"act_2\"", "\"act_1\",\"act_1\""),
        ("\"modifiers\":[]", "\"modifiers\":[\"z\",\"a\"]"),
    ] {
        let mut changed = original.clone();
        changed["gameplay"]["selected_context"] = context_variant(from, to);
        assert!(Manifest::parse_private(&bytes(&changed)).is_err(), "{from}");
    }
}

#[test]
fn experiment_and_occurrence_identity_have_separate_lifetimes() {
    let original = document();
    let baseline = parse(&original);
    for (path, value) in [
        ("/experiment/provider", json!("other-provider")),
        ("/experiment/model", json!("other-model")),
        (
            "/experiment/provider_revision",
            json!({"status":"known","value":"revision-2"}),
        ),
        (
            "/experiment/model_revision",
            json!({"status":"unavailable"}),
        ),
        ("/experiment/prompt_digest", json!("c".repeat(64))),
        ("/experiment/workflow_digest", json!("c".repeat(64))),
        ("/experiment/context_digest", json!("c".repeat(64))),
        ("/experiment/tool_policy_digest", json!("c".repeat(64))),
        ("/experiment/inference_parameters/temperature", json!("0.9")),
        (
            "/experiment/inference_seed",
            json!({"status":"requested","value":"42","guarantee":"best_effort"}),
        ),
        ("/experiment/budgets/max_decisions", json!(101)),
        ("/experiment/budgets/max_tokens", json!(10001)),
        ("/experiment/budgets/max_duration_ms", json!(60001)),
        ("/experiment/evaluator_revision", json!("other-evaluator")),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(path).unwrap() = value;
        let candidate = parse(&changed);
        assert_eq!(
            baseline.configuration_digest_private(),
            candidate.configuration_digest_private()
        );
        assert_ne!(
            baseline.experiment_digest_private(),
            candidate.experiment_digest_private(),
            "{path}"
        );
        assert_eq!(baseline.compare(&candidate), vec![Mismatch::Experiment]);
    }
    let first = baseline.plan_trial(&bytes(&occurrence())).unwrap();
    let mut other = occurrence();
    other["run_id"] = json!("run-2");
    other["operation_id"] = json!("op-seed-2");
    other["created_at_unix_ms"] = json!(100);
    let second = baseline.plan_trial(&bytes(&other)).unwrap();
    assert_eq!(
        first.manifest().artifact_digest_private(),
        second.manifest().artifact_digest_private()
    );
    assert_ne!(
        first.export_private().unwrap(),
        second.export_private().unwrap()
    );
}

#[test]
fn strict_parser_rejects_missing_null_unknown_duplicate_and_oversized_inputs() {
    let original = document();
    for path in ["/gameplay", "/experiment"] {
        for key in original.pointer(path).unwrap().as_object().unwrap().keys() {
            let mut missing = original.clone();
            missing
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                Manifest::parse_private(&bytes(&missing)).is_err(),
                "{path}/{key}"
            );
            let mut null = original.clone();
            null.pointer_mut(path).unwrap()[key] = Value::Null;
            assert!(
                Manifest::parse_private(&bytes(&null)).is_err(),
                "{path}/{key}"
            );
        }
    }
    for path in [
        "",
        "/gameplay",
        "/gameplay/platform",
        "/experiment",
        "/experiment/budgets",
    ] {
        let mut changed = original.clone();
        changed.pointer_mut(path).unwrap()["unknown_required_semantics"] = json!(true);
        assert!(Manifest::parse_private(&bytes(&changed)).is_err());
    }
    let text = String::from_utf8(bytes(&original)).unwrap();
    for (from, to) in [
        ("\"version\":", "\"version\":\"ignored\",\"version\":"),
        ("\"os\":", "\"os\":\"ignored\",\"os\":"),
        (
            "\"temperature\":",
            "\"temperature\":\"ignored\",\"temperature\":",
        ),
        (
            "\"synthetic-assembly\":",
            "\"synthetic-assembly\":\"ignored\",\"synthetic-assembly\":",
        ),
    ] {
        assert!(Manifest::parse_private(text.replace(from, to).as_bytes()).is_err());
    }
    assert_eq!(
        Manifest::parse_private(&vec![b' '; MAX_MANIFEST_BYTES + 1]).unwrap_err(),
        ManifestError::TooLarge
    );
    assert!(Manifest::parse_private(format!("{text} null").as_bytes()).is_err());
    let mut version = original.clone();
    version["version"] = json!("ascension.benchmark-manifest.v2");
    assert_eq!(
        Manifest::parse_private(&bytes(&version)).unwrap_err(),
        ManifestError::UnsupportedVersion
    );
}

#[test]
fn positive_bounds_and_explicit_unavailable_guarantees_never_become_unlimited() {
    let mut input = document();
    input["experiment"]["budgets"] =
        json!({"max_decisions":1000000,"max_tokens":1000000000,"max_duration_ms":604800000});
    input["experiment"]["inference_parameters"] = (0..32)
        .map(|i| (format!("p{i}"), json!("x".repeat(128))))
        .collect::<serde_json::Map<_, _>>()
        .into();
    parse(&input);
    for (path, value) in [
        ("/experiment/budgets/max_decisions", json!(0)),
        ("/experiment/budgets/max_decisions", json!(1000001)),
        ("/experiment/budgets/max_tokens", json!(1000000001_u64)),
        ("/experiment/budgets/max_duration_ms", json!(604800001)),
        (
            "/experiment/inference_seed",
            json!({"status":"requested","value":"42","guarantee":"deterministic"}),
        ),
        ("/gameplay/effective_seed", json!("x".repeat(65))),
        ("/gameplay/effective_seed", json!("hidden\nseed")),
        (
            "/gameplay/selected_context/context_digest",
            json!("0".repeat(64)),
        ),
        (
            "/gameplay/profile_artifact/baseline_digest",
            json!("0".repeat(64)),
        ),
        (
            "/gameplay/profile_artifact/reference",
            json!("/private/path"),
        ),
        (
            "/gameplay/profile_artifact/reference",
            json!("file:///private/path"),
        ),
    ] {
        let mut changed = input.clone();
        *changed.pointer_mut(path).unwrap() = value;
        assert!(Manifest::parse_private(&bytes(&changed)).is_err(), "{path}");
    }
    input["experiment"]["inference_parameters"]["extra"] = json!("1");
    assert!(Manifest::parse_private(&bytes(&input)).is_err());
}

#[test]
fn public_projection_and_debug_never_expose_private_content_or_exact_digests() {
    let manifest = parse(&document());
    let public = serde_json::to_value(manifest.public_projection(&[7; 32]).unwrap()).unwrap();
    let object = public.as_object().unwrap();
    assert_eq!(object.len(), 3);
    assert_eq!(object["evidence"], "declared_inputs_only");
    assert!(
        object["reference"]
            .as_str()
            .unwrap()
            .starts_with("benchmark-h1:")
    );
    let output = format!("{public} {manifest:?}");
    for secret in [
        "ironclad-42",
        "fresh-standard-comparison",
        "profile-artifact-1",
        "synthetic-provider",
        manifest.configuration_digest_private(),
        manifest.experiment_digest_private(),
        manifest.artifact_digest_private(),
    ] {
        assert!(!output.contains(secret), "{secret}");
    }
    assert_ne!(
        manifest.public_projection(&[7; 32]).unwrap(),
        manifest.public_projection(&[8; 32]).unwrap()
    );
    assert!(manifest.public_projection(&[7; 31]).is_err());
    assert!(manifest.public_projection(&[7; 65]).is_err());
    let mut changed = document();
    changed["gameplay"]["effective_seed"] = json!("other-seed");
    assert_ne!(
        manifest.public_projection(&[7; 32]).unwrap(),
        parse(&changed).public_projection(&[7; 32]).unwrap()
    );
}

#[test]
fn seed_bytes_inventory_and_context_array_limits_are_checked_without_clamping() {
    let mut input = document();
    input["gameplay"]["effective_seed"] = json!("é".repeat(32));
    input["gameplay"]["assembly_hashes"] = (0..32)
        .map(|i| (format!("assembly-{i}"), json!("c".repeat(64))))
        .collect::<serde_json::Map<_, _>>()
        .into();
    parse(&input);
    input["gameplay"]["effective_seed"] = json!("é".repeat(33));
    assert!(Manifest::parse_private(&bytes(&input)).is_err());
    input["gameplay"]["effective_seed"] = json!("x".repeat(64));
    input["gameplay"]["assembly_hashes"]["extra"] = json!("d".repeat(64));
    assert!(Manifest::parse_private(&bytes(&input)).is_err());
    for count in [32, 33] {
        let modifiers: Vec<_> = (0..count).map(|i| format!("modifier-{i:02}")).collect();
        let literal = serde_json::to_string(&modifiers).unwrap();
        let mut changed = document();
        changed["gameplay"]["selected_context"] =
            context_variant("\"modifiers\":[]", &format!("\"modifiers\":{literal}"));
        assert_eq!(
            Manifest::parse_private(&bytes(&changed)).is_ok(),
            count == 32
        );
    }
    for count in [8, 9] {
        let acts: Vec<_> = (0..count).map(|i| format!("act-{i}")).collect();
        let literal = serde_json::to_string(&acts).unwrap();
        let mut changed = document();
        changed["gameplay"]["selected_context"] = context_variant(
            "\"acts\":[\"act_1\",\"act_2\",\"act_3\",\"act_4\"]",
            &format!("\"acts\":{literal}"),
        );
        assert_eq!(
            Manifest::parse_private(&bytes(&changed)).is_ok(),
            count == 8
        );
    }
}
