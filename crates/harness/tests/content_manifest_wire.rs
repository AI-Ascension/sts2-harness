// SPDX-License-Identifier: MIT
//! Compile both in-org consumers of the additive frame, without an external Exo checkout.
#![allow(dead_code)]

#[path = "../../../experiments/exo-agent/bridge/src/lookup_manifest.rs"]
mod lookup_manifest;
#[path = "../../../experiments/exo-agent/bridge/src/lookup_wire.rs"]
mod lookup_wire;

use serde_json::{Value, json};
use sts2_harness::exo_bridge_configuration as config;
use sts2_harness::exo_lookup_process::ExoLookupProfile;
use sts2_harness::exo_lookup_wire::{EXO_LOOKUP_MANIFEST_WIRE, ExoLookupFrame};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn both_consumers_share_every_new_golden_and_negative_frame() -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/fixtures/exo-lookup-content-manifest-v1");
    for (file, valid) in [
        ("valid/manifest-request.json", true),
        ("valid/manifest-read-bound.json", true),
        ("invalid/legacy-wire.json", false),
        ("invalid/model-scope.json", false),
        ("invalid/history-wire.json", false),
    ] {
        let bytes = std::fs::read(root.join(file))?;
        let host = ExoLookupFrame::parse(&bytes);
        let executor = lookup_wire::decode_frame(&bytes);
        assert_eq!(host.is_ok(), valid, "host {file}");
        assert_eq!(executor.is_ok(), valid, "executor {file}");
        if valid {
            let host = host?;
            let executor = executor?;
            assert_eq!(
                serde_json::to_value(&host)?,
                serde_json::to_value(&executor)?
            );
            assert_eq!(
                lookup_wire::version_for_payload(&executor.payload),
                EXO_LOOKUP_MANIFEST_WIRE
            );
            assert!(ExoLookupProfile::ContentManifest.permits_frame(&host));
            for profile in [
                ExoLookupProfile::Terminal,
                ExoLookupProfile::Bootstrap,
                ExoLookupProfile::History,
            ] {
                assert!(!profile.permits_frame(&host), "{profile:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn executor_closes_selector_identity_bounds_and_feedback_correlation() -> TestResult {
    for name in [
        "sts2_lookup_query",
        "sts2_lookup_read",
        "sts2_lookup_bootstrap",
        "sts2_lookup_history",
        "sts2_lookup_content_manifest",
        "sts2_lookup_manifest_read",
    ] {
        assert!(
            lookup_wire::owned_tool_name(name),
            "native event receipt: {name}"
        );
    }
    for name in [
        "shell",
        "sts2.game_information_content_manifest",
        "manifest",
        "sts2_lookup_manifest_write",
    ] {
        assert!(!lookup_wire::owned_tool_name(name));
    }
    assert!(lookup_manifest::payload(json!({"operation_id":"manifest-1"}), false).is_ok());
    for arguments in [
        json!({"operation_id":""}),
        json!({"operation_id":"../private"}),
        json!({"operation_id":"x".repeat(65)}),
        json!({"operation_id":"m","locale":"en"}),
        json!({"operation_id":"m","content_manifest_id":"foreign"}),
    ] {
        assert!(lookup_manifest::payload(arguments, false).is_err());
    }
    assert!(lookup_manifest::payload(json!({"record_ordinal":31,"offset":131072}), true).is_ok());
    for arguments in [
        json!({"record_ordinal":32,"offset":0}),
        json!({"record_ordinal":0,"offset":131073}),
        json!({"record_ordinal":0,"offset":-1}),
        json!({"record_ordinal":0,"offset":0,"source":"foreign"}),
    ] {
        assert!(lookup_manifest::payload(arguments, true).is_err());
    }
    let frame = json!({"wire_version":EXO_LOOKUP_MANIFEST_WIRE,"request_id":"request-1",
        "turn_id":"turn-1","sequence":1,"payload":{"kind":"feedback","value":{"manifest_record_ordinal":0}}});
    let bytes = serde_json::to_vec(&frame)?;
    assert!(
        lookup_wire::feedback_for_version(
            &bytes,
            "request-1",
            "turn-1",
            1,
            EXO_LOOKUP_MANIFEST_WIRE
        )
        .is_ok()
    );
    for (request, turn, sequence, pin) in [
        ("wrong", "turn-1", 1, EXO_LOOKUP_MANIFEST_WIRE),
        ("request-1", "wrong", 1, EXO_LOOKUP_MANIFEST_WIRE),
        ("request-1", "turn-1", 2, EXO_LOOKUP_MANIFEST_WIRE),
        ("request-1", "turn-1", 1, lookup_wire::VERSION),
    ] {
        assert!(lookup_wire::feedback_for_version(&bytes, request, turn, sequence, pin).is_err());
    }
    Ok(())
}

fn loaded() -> config::Loaded {
    config::Loaded {
        config: config::Configuration {
            schema: "sts2.exo-lookup-config-v1".into(),
            executor: "/executor".into(),
            executor_sha256: "0".repeat(64),
            source_root: "/source".into(),
            extension: "/extension".into(),
            extension_sha256: "0".repeat(64),
            node: "/node".into(),
            node_sha256: "0".repeat(64),
            model: "o3-pro".into(),
            endpoint: "http://127.0.0.1:8080".into(),
        },
        digest: "0".repeat(64),
        private_state: config::PrivateStateProfile::LegacyV1,
    }
}

#[test]
fn explicit_new_descriptor_preserves_old_surfaces_and_declares_real_limits() -> TestResult {
    let loaded = loaded();
    let old = loaded.lookup_history_description()?;
    let descriptor = loaded.lookup_manifest_description()?;
    assert_eq!(
        descriptor["schema"],
        "sts2.exo-lookup-capability-v4-content-manifest"
    );
    assert_eq!(descriptor["wire_version"], EXO_LOOKUP_MANIFEST_WIRE);
    let tools = descriptor["tools"].as_array().ok_or("tools")?;
    let old_tools = old["tools"].as_array().ok_or("old tools")?;
    assert_eq!(&tools[..old_tools.len()], old_tools);
    assert_eq!(tools.len(), 6);
    assert_eq!(
        &tools[4..],
        &[
            json!("sts2_lookup_content_manifest"),
            json!("sts2_lookup_manifest_read")
        ]
    );
    assert_eq!(descriptor["content_manifest"]["max_response_bytes"], 131072);
    assert_eq!(
        descriptor["content_manifest"]["complete_retention_required"],
        true
    );
    for key in ["decisions", "max_tool_round_trips", "max_model_writes"] {
        assert_eq!(descriptor[key], old[key]);
    }
    for older in [
        loaded.lookup_description()?,
        loaded.lookup_bootstrap_description()?,
        old,
    ] {
        assert!(
            older["tools"]
                .as_array()
                .ok_or("tools")?
                .iter()
                .all(|name| !name.as_str().is_some_and(|name| name.contains("manifest")))
        );
        assert!(older.get("content_manifest").is_none());
    }
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../conformance/fixtures/exo-lookup-content-manifest-v1/valid/manifest-request.json",
    ))?;
    let value: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        value["payload"],
        json!({"kind":"content_manifest","operation_id":"manifest-1"})
    );
    Ok(())
}
