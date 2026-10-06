// SPDX-License-Identifier: MIT
#![allow(clippy::expect_used)]

use super::{PrivateStateProfile, decode_profile};
use serde_json::{Value, json};

fn common(schema: &str) -> Value {
    json!({
        "schema": schema,
        "executor": "/opt/sts2/executor",
        "executor_sha256": "a".repeat(64),
        "source_root": "/opt/sts2/source",
        "extension": "/opt/sts2/extension.js",
        "extension_sha256": "b".repeat(64),
        "node": "/opt/sts2/bin/node",
        "node_sha256": "c".repeat(64),
        "model": "o3-pro",
        "endpoint": "https://api.openai.com/v1"
    })
}

fn policy() -> Value {
    json!({
        "state_root": "/var/lib/sts2-test/state",
        "cache_root": "/var/lib/sts2-test/cache",
        "temp_root": "/var/lib/sts2-test/temp",
        "quota_bytes": 1048576,
        "max_retention_days": 1,
        "permissions_octal": 448
    })
}

#[test]
fn v1_keeps_its_closed_lexical_compatibility_profile() {
    let bytes = serde_json::to_vec(&common("sts2.exo-one-shot-config-v1"));
    let decoded = decode_profile(&bytes.unwrap_or_default(), false);
    assert!(
        decoded.is_ok(),
        "the original closed v1 config remains valid"
    );
    if let Ok((config, profile)) = decoded {
        assert_eq!(config.schema, "sts2.exo-one-shot-config-v1");
        assert_eq!(profile, PrivateStateProfile::LegacyV1);
    }

    let mut extra = common("sts2.exo-one-shot-config-v1");
    extra["private_state"] = policy();
    let bytes = serde_json::to_vec(&extra).unwrap_or_default();
    assert!(decode_profile(&bytes, false).is_err());
}

#[test]
fn guarded_v2_is_closed_versioned_and_requires_a_valid_explicit_policy() {
    let mut config = common("sts2.exo-one-shot-config-v2");
    config["private_state"] = policy();
    let bytes = serde_json::to_vec(&config).unwrap_or_default();
    let (decoded, profile) = decode_profile(&bytes, false).expect("guarded v2 config is valid");
    assert_eq!(decoded.schema, "sts2.exo-one-shot-config-v2");
    assert!(matches!(profile, PrivateStateProfile::GuardedV2(_)));

    let missing = common("sts2.exo-one-shot-config-v2");
    assert!(decode_profile(&serde_json::to_vec(&missing).unwrap_or_default(), false).is_err());

    config["unexpected"] = json!(true);
    assert!(decode_profile(&serde_json::to_vec(&config).unwrap_or_default(), false).is_err());

    let mut malformed = common("sts2.exo-one-shot-config-v2");
    malformed["private_state"] = policy();
    malformed["private_state"]["permissions_octal"] = json!(0o777);
    assert!(decode_profile(&serde_json::to_vec(&malformed).unwrap_or_default(), false).is_err());
}

#[test]
fn lookup_and_run_v2_schemas_cannot_be_interchanged_or_downgraded() {
    let mut run = common("sts2.exo-one-shot-config-v2");
    run["private_state"] = policy();
    let bytes = serde_json::to_vec(&run).unwrap_or_default();
    assert!(decode_profile(&bytes, true).is_err());

    run["schema"] = json!("sts2.exo-lookup-config-v2");
    assert!(decode_profile(&serde_json::to_vec(&run).unwrap_or_default(), false).is_err());
    assert!(decode_profile(&serde_json::to_vec(&run).unwrap_or_default(), true).is_ok());
}

#[test]
fn duplicate_config_keys_are_rejected_before_profile_selection() {
    let bytes =
        br#"{"schema":"sts2.exo-one-shot-config-v1","schema":"sts2.exo-one-shot-config-v2"}"#;
    assert!(decode_profile(bytes, false).is_err());
}
