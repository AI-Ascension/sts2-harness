// SPDX-License-Identifier: MIT

#![allow(clippy::panic)]

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sts2_harness::seed_binding::{MAX_SEED_BYTES, canonicalize_seed};

use super::SeedTransportConfig;
use super::validation::{context_digest, validate_context, validate_plan};

impl SeedTransportConfig {
    pub(crate) fn fixture_with_context_mismatch(kind: &str) -> Self {
        let mut config = Self::fixture_for_tests("ironclad-42", "op-seed-1");
        match kind {
            "context" => config.context.selection_policy = String::from("alternate_policy"),
            "profile" => {
                config.context.profile_baseline.identity = String::from("other-profile");
            }
            "game_compatibility" => {
                config.context.compatibility.game.identity = String::from("sts2-game/v0.107.2");
            }
            "mod_compatibility" => {
                config.context.compatibility.mod_identity.identity = String::from("other-game-mod");
            }
            "context_digest" => config.context.context_digest = "b".repeat(64),
            _ => config.context.selection_policy = String::from("alternate_policy"),
        }
        config
    }
}
use super::{PlanDocument, SelectedContext};

#[test]
fn concrete_standard_ironclad_context_matches_protocol_golden_digest() {
    let context: SelectedContext = serde_json::from_str(
            r#"{"context_id":"standard/ironclad/asc0/fresh","game_mode":"standard","character":"ironclad","ascension":0,"modifiers":[],"acts":["act_1","act_2","act_3","act_4"],"selection_policy":"standard_default","profile_baseline":{"kind":"fresh","identity":"fresh-standard-comparison","digest":"4581aaf95348126550cdf3b73ec46b39d447523cf7cb35aec71c2842d1945031"},"save_policy":"disabled","compatibility":{"game":{"identity":"sts2-game/v0.107.1","digest":"2db9d9f665c776c2324c7f98134b900a8b3332f32148ea1cb84063d52db94ff4"},"mod":{"identity":"ai-ascension/sts2-game-mod","digest":"0c6e7bbb54996222a4894de999fc860decc360f9936c9bd5a7382d97b361bb7e"}},"context_digest":"d57563180f198b73970510427981504a9df10c62931577e601f9dcce6275fbe9"}"#,
        )
        .map_err(|error| error.to_string())
        .unwrap_or_else(|error| panic!("golden context must parse: {error}"));
    validate_context(&context).unwrap_or_else(|error| panic!("golden context invalid: {error}"));
    assert_eq!(
        context_digest(&context).unwrap_or_default(),
        context.context_digest
    );
}

#[test]
fn seeded_start_arguments_match_seeded_run_v1_mcp_catalog() {
    let config = SeedTransportConfig::fixture_for_tests("S2TSWF0651", "seed-start-test");
    let arguments = config.start_arguments("instance-1", "mcp-session-1", "lease-1", 3, 7);
    let object = arguments
        .as_object()
        .unwrap_or_else(|| panic!("seeded start arguments must be an object"));
    let mut actual = object.keys().map(String::as_str).collect::<Vec<_>>();
    actual.sort_unstable();
    let mut expected = vec![
        "generation",
        "instance_id",
        "lease_epoch",
        "lease_id",
        "mcp_session_id",
        "operation_id",
        "run_mode",
        "seed",
        "selected_context",
    ];
    expected.sort_unstable();
    assert_eq!(actual, expected);
    assert_eq!(
        object.get("seed").and_then(Value::as_str),
        Some("S2TSWF0651")
    );
    assert!(!object.contains_key("requested_seed"));
    assert!(!object.contains_key("context_digest"));
    assert!(!object.contains_key("plan_digest"));
    assert!(!object.contains_key("entry_ordinal"));
    assert_eq!(
        object
            .get("selected_context")
            .and_then(|value| value.get("context_digest"))
            .and_then(Value::as_str),
        Some(config.context_digest())
    );
}

#[test]
fn plan_requires_explicit_contiguous_ordinals_and_bounded_seed() {
    let valid = PlanDocument {
        entries: vec![
            super::PlanEntry {
                ordinal: 0,
                requested_seed: String::from("S2TSWF0651"),
            },
            super::PlanEntry {
                ordinal: 1,
                requested_seed: String::from("S2TSWF0652"),
            },
        ],
    };
    assert!(validate_plan(&valid).is_ok());
    let mut invalid = valid.clone();
    invalid.entries[1].ordinal = 3;
    assert!(validate_plan(&invalid).is_err());
    invalid.entries[1].ordinal = 1;
    invalid.entries[1].requested_seed = "x".repeat(65);
    assert!(validate_plan(&invalid).is_err());
}

#[test]
fn plan_seed_validation_matches_canonical_bytes_without_normalizing() {
    let exact_ascii = "a".repeat(MAX_SEED_BYTES);
    let oversized_ascii = "a".repeat(MAX_SEED_BYTES + 1);
    let exact_multibyte = format!("{}ab", "é".repeat(31));
    let oversized_multibyte = format!("{}abc", "é".repeat(31));
    assert_eq!(exact_multibyte.len(), MAX_SEED_BYTES);
    assert_eq!(oversized_multibyte.len(), MAX_SEED_BYTES + 1);

    let cases = [
        ("empty", String::new(), false),
        ("leading ASCII space", String::from(" seed"), false),
        ("trailing ASCII space", String::from("seed "), false),
        (
            "leading Unicode whitespace",
            String::from("\u{2003}seed"),
            false,
        ),
        (
            "trailing Unicode whitespace",
            String::from("seed\u{2003}"),
            false,
        ),
        (
            "control character",
            String::from("seed\u{0001}value"),
            false,
        ),
        ("NUL", String::from("seed\0value"), false),
        ("64 ASCII bytes", exact_ascii, true),
        ("65 ASCII bytes", oversized_ascii, false),
        ("64 UTF-8 bytes", exact_multibyte, true),
        ("65 UTF-8 bytes", oversized_multibyte, false),
        (
            "interior ASCII whitespace",
            String::from("seed with space"),
            true,
        ),
        (
            "interior Unicode whitespace",
            String::from("seed\u{2003}middle"),
            true,
        ),
    ];

    for (case, requested_seed, expected_valid) in cases {
        let library = canonicalize_seed(&requested_seed);
        let plan = super::PlanDocument {
            entries: vec![super::PlanEntry {
                ordinal: 0,
                requested_seed: requested_seed.clone(),
            }],
        };
        let transport = validate_plan(&plan);

        assert_eq!(transport.is_ok(), library.is_ok(), "{case}");
        assert_eq!(transport.is_ok(), expected_valid, "{case}");
        assert_eq!(plan.entries[0].ordinal, 0, "{case}");
        assert_eq!(plan.entries[0].requested_seed, requested_seed, "{case}");
        if expected_valid {
            assert_eq!(library.as_deref(), Ok(requested_seed.as_str()), "{case}");
        } else {
            assert_eq!(
                transport.err().as_deref(),
                Some("requested seed is empty, unsafe, or oversized"),
                "{case}"
            );
            assert!(library.is_err(), "{case}");
        }
    }
}

#[test]
fn reservation_restarts_in_reconcile_only_mode_and_rejects_conflict() {
    let config = SeedTransportConfig {
        plan_digest: "a".repeat(64),
        entry_ordinal: 0,
        requested_seed: String::from("S2TSWF0651"),
        operation_id: String::from("seed-start-test"),
        run_mode: String::from("seeded_training"),
        verify_idempotency: false,
        context: SelectedContext {
            context_id: String::from("standard/ironclad/asc0/fresh"),
            game_mode: String::from("standard"),
            character: String::from("ironclad"),
            ascension: 0,
            modifiers: Vec::new(),
            acts: vec![String::from("act_1")],
            selection_policy: String::from("standard_default"),
            profile_baseline: super::ProfileBaseline {
                kind: String::from("fresh"),
                identity: String::from("test-profile"),
                digest: "b".repeat(64),
            },
            save_policy: String::from("enabled"),
            compatibility: super::Compatibility {
                game: super::IdentityDigest {
                    identity: String::from("game"),
                    digest: "c".repeat(64),
                },
                mod_identity: super::IdentityDigest {
                    identity: String::from("mod"),
                    digest: "d".repeat(64),
                },
            },
            context_digest: String::new(),
        },
    };
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let path = std::env::temp_dir().join(format!(
        "sts2-seed-reservation-{}-{suffix}.json",
        std::process::id()
    ));
    let first = config.reserve_at(path.clone(), 7);
    assert!(first.is_ok());
    let first = match first {
        Ok(value) => value,
        Err(_) => return,
    };
    assert!(!first.resumed);
    assert!(first.mark_unknown(&config).is_ok());
    let resumed = config.reserve_at(path.clone(), 99);
    assert!(resumed.is_ok());
    assert!(resumed.is_ok_and(|value| value.resumed && value.request_generation() == 7));
    let mut conflicting = config.clone();
    conflicting.operation_id = String::from("different-operation");
    assert!(conflicting.reserve_at(path.clone(), 7).is_err());
    let _ = std::fs::remove_file(path);
}
