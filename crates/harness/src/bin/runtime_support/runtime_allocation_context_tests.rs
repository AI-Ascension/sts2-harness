// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::super::super::config::RuntimeConfig;
use super::{ALLOCATION_SCHEMA_DIGEST, validate};

const INSTANCE_ID: &str = "00000000-0000-4000-8000-000000000002";
const STATIC_LEASE: &str = "00000000-0000-4000-8000-000000000099";
const ACQUIRED_LEASE: &str = "00000000-0000-4000-8000-000000000006";

fn config() -> RuntimeConfig {
    RuntimeConfig {
        seed_transport: None,
        gateway_address: "127.0.0.1:1".into(),
        gateway_token: "synthetic-token".into(),
        mcp_binary: "unused".into(),
        runtime_profile: "runtime-v3-gameplay".into(),
        instance_id: INSTANCE_ID.into(),
        caller_id: "harness".into(),
        session_id: "session-1".into(),
        lease_id: STATIC_LEASE.into(),
        lease_epoch: 1,
        mcp_session_id: "mcp-session-1".into(),
        run_id: "run-1".into(),
        episode_id: "episode-1".into(),
        trajectory_id: "trajectory-1".into(),
        trace_id: "trace-1".into(),
        artifact_id: "artifact-1".into(),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: Vec::new(),
    }
}

fn authority() -> Value {
    json!({
        "contract": "watchdog-runtime-allocation-v1",
        "schema_digest": ALLOCATION_SCHEMA_DIGEST,
        "context": {
            "deployment_id": "00000000-0000-4000-8000-000000000001",
            "instance_id": INSTANCE_ID,
            "instance_incarnation": "00000000-0000-4000-8000-000000000003",
            "boot_id": "00000000-0000-4000-8000-000000000004",
            "authority_generation": 7,
            "lease_id": ACQUIRED_LEASE,
            "lease_epoch": 3
        },
        "current_fence": {
            "host_fence_id": "00000000-0000-4000-8000-000000000005",
            "deployment_id": "00000000-0000-4000-8000-000000000001",
            "instance_id": INSTANCE_ID,
            "instance_incarnation": "00000000-0000-4000-8000-000000000003",
            "boot_id": "00000000-0000-4000-8000-000000000004",
            "authority_generation": 7,
            "fence_generation": 4,
            "created_at": "2026-09-07T00:00:01Z"
        }
    })
}

fn allocation(with_authority: bool) -> Value {
    let mut value = json!({
        "status": "allocated",
        "instance_id": INSTANCE_ID,
        "caller_id": "harness",
        "session_id": "session-1",
        "lease_id": STATIC_LEASE,
        "lease_epoch": 1,
        "transport": "attached-loopback"
    });
    if with_authority {
        value["lease_id"] = json!(ACQUIRED_LEASE);
        value["lease_epoch"] = json!(3);
        value["recovery_authority"] = authority();
    }
    value
}

fn source_fixture(path: &str) -> Result<Value, String> {
    serde_json::from_str(path).map_err(|error| format!("source fixture is not JSON: {error}"))
}

#[test]
fn acquired_authority_replaces_only_the_current_lease() -> Result<(), String> {
    let mut config = config();
    let parsed = validate(&allocation(true), &config)?;
    assert_eq!(parsed.lease_id, ACQUIRED_LEASE);
    assert_eq!(parsed.lease_epoch, 3);
    assert!(parsed.recovery_authority.is_some());
    parsed.apply_current_lease(&mut config);
    assert_eq!(config.lease_id, ACQUIRED_LEASE);
    assert_eq!(config.lease_epoch, 3);
    Ok(())
}

#[test]
fn non_recovery_allocation_keeps_the_static_lease_path() -> Result<(), String> {
    let mut config = config();
    let parsed = validate(&allocation(false), &config)?;
    assert!(parsed.recovery_authority.is_none());
    parsed.apply_current_lease(&mut config);
    assert_eq!(config.lease_id, STATIC_LEASE);
    assert_eq!(config.lease_epoch, 1);
    Ok(())
}

#[test]
fn recovery_credentials_require_the_authority_extension() -> Result<(), String> {
    let mut config = config();
    config
        .recovery_environment
        .push((String::from("STS2_RECOVERY_TOKEN"), String::from("token")));
    let error = validate(&allocation(false), &config)
        .err()
        .ok_or_else(|| String::from("authority must be required"))?;
    assert_eq!(error, "recovery allocation omitted recovery_authority");
    Ok(())
}

#[test]
fn authority_rejects_context_fence_mismatch() -> Result<(), String> {
    let mut value = allocation(true);
    value["recovery_authority"]["current_fence"]["boot_id"] =
        json!("00000000-0000-4000-8000-000000000007");
    let error = validate(&value, &config())
        .err()
        .ok_or_else(|| String::from("mixed authority must fail"))?;
    assert_eq!(
        error,
        "recovery allocation current fence does not match its context"
    );
    Ok(())
}

#[test]
fn authority_rejects_digest_and_unknown_fields() -> Result<(), String> {
    let mut bad_digest = allocation(true);
    bad_digest["recovery_authority"]["schema_digest"] = json!("0".repeat(64));
    assert!(validate(&bad_digest, &config()).is_err());

    let mut unknown = allocation(true);
    unknown["recovery_authority"]["unexpected"] = json!(true);
    let error = validate(&unknown, &config())
        .err()
        .ok_or_else(|| String::from("unknown field must fail"))?;
    assert_eq!(error, "recovery_authority has unexpected fields");
    Ok(())
}

#[test]
fn exact_source_fixtures_cover_valid_and_context_mismatch_cases() -> Result<(), String> {
    let mut valid = allocation(false);
    valid["lease_id"] = json!(ACQUIRED_LEASE);
    valid["lease_epoch"] = json!(3);
    valid["recovery_authority"] = source_fixture(include_str!(
        "../../../../../contract-artifact/runtime-allocation-v1/fixtures/valid/recovery-authority.json"
    ))?;
    assert!(validate(&valid, &config()).is_ok());

    let mut invalid = valid;
    invalid["recovery_authority"] = source_fixture(include_str!(
        "../../../../../contract-artifact/runtime-allocation-v1/fixtures/semantic-invalid/fence-context-mismatch.json"
    ))?;
    let error = validate(&invalid, &config())
        .err()
        .ok_or_else(|| String::from("context mismatch fixture was accepted"))?;
    assert_eq!(
        error,
        "recovery allocation current fence does not match its context"
    );
    Ok(())
}

#[test]
fn imported_schema_manifest_and_fixture_digests_stay_consistent() -> Result<(), String> {
    let manifest: Value = source_fixture(include_str!(
        "../../../../../contract-artifact/runtime-allocation-v1/manifest.json"
    ))?;
    let manifest_digest = manifest["schema_digest"]
        .as_str()
        .ok_or_else(|| String::from("allocation manifest omitted schema_digest"))?;
    let schema_digest = format!(
        "{:x}",
        Sha256::digest(include_bytes!(
            "../../../../../contract-artifact/runtime-allocation-v1/frame.schema.json"
        ))
    );
    assert_eq!(manifest_digest, schema_digest);
    assert_eq!(manifest_digest, ALLOCATION_SCHEMA_DIGEST);
    for value in [
        source_fixture(include_str!(
            "../../../../../contract-artifact/runtime-allocation-v1/fixtures/valid/recovery-authority.json"
        ))?,
        source_fixture(include_str!(
            "../../../../../contract-artifact/runtime-allocation-v1/fixtures/semantic-invalid/fence-context-mismatch.json"
        ))?,
    ] {
        assert_eq!(value["schema_digest"].as_str(), Some(manifest_digest));
    }
    Ok(())
}

#[test]
fn status_and_lease_bindings_are_required() -> Result<(), String> {
    for status in [Value::Null, json!("ready"), json!("allocated ")] {
        let mut value = allocation(true);
        value["status"] = status;
        let error = validate(&value, &config())
            .err()
            .ok_or_else(|| String::from("invalid allocation status was accepted"))?;
        assert_eq!(error, "gateway allocation status was not allocated");
    }

    let mut mismatched = allocation(true);
    mismatched["lease_epoch"] = json!(4);
    let error = validate(&mismatched, &config())
        .err()
        .ok_or_else(|| String::from("mismatched response lease was accepted"))?;
    assert_eq!(
        error,
        "gateway allocation authority does not match its lease"
    );
    Ok(())
}

#[test]
fn timestamp_validation_uses_the_gregorian_calendar() -> Result<(), String> {
    for timestamp in [
        "2026-02-29T00:00:01Z",
        "2026-04-31T00:00:01Z",
        "2026-06-31T00:00:01Z",
        "2026-13-01T00:00:01Z",
        "2026-01-00T00:00:01Z",
        "2026-01-01T24:00:01Z",
        "2026-01-01T00:00:60Z",
    ] {
        let mut value = allocation(true);
        value["recovery_authority"]["current_fence"]["created_at"] = json!(timestamp);
        if validate(&value, &config()).is_ok() {
            return Err(format!(
                "invalid calendar timestamp was accepted: {timestamp}"
            ));
        }
    }
    let mut leap = allocation(true);
    leap["recovery_authority"]["current_fence"]["created_at"] =
        json!("2024-02-29T00:00:01.123456789Z");
    assert!(validate(&leap, &config()).is_ok());
    Ok(())
}
