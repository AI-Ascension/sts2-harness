// SPDX-License-Identifier: MIT

#![allow(clippy::panic)]

use std::time::{SystemTime, UNIX_EPOCH};

use super::validation::{context_digest, validate_context, validate_plan};
use super::{PlanDocument, SeedTransportConfig, SelectedContext};

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
