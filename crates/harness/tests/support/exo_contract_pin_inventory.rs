// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn manifest_pin_inventory_lists_existing_required_consumers() {
    let manifest: serde_json::Value =
        serde_json::from_str(exo_bridge_manifest()).expect("manifest is JSON");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let pins = manifest["pin_locations"]
        .as_array()
        .expect("pin_locations is an array")
        .iter()
        .map(|value| value.as_str().expect("pin location is a string"))
        .collect::<Vec<_>>();
    for path in &pins {
        assert!(
            root.join(path).exists(),
            "recorded Exo pin location is missing: {path}"
        );
    }
    for required in [
        "crates/harness/src/exo/contract/mod.rs",
        "crates/harness/src/exo/contract/identity.rs",
        "crates/harness/src/exo/contract/preflight.rs",
        "crates/harness/src/bin/runtime_support/runtime_v3_settings.rs",
        "crates/harness/src/bin/runtime_support/runtime_v3_durable_support.rs",
        "crates/harness/src/bin/sts2-harness-exo.rs",
        "experiments/exo-agent/config.example.toml",
        "experiments/exo-agent/extension/package.json",
        "experiments/exo-agent/extension/src/index.ts",
        "docs/decisions/0017-exo-executor-bridge-contract.md",
        "docs/decisions/0018-exo-one-shot-executor-package.md",
        "docs/evidence/exo-extension-real-spike-20260914.md",
        "protocol-artifact/exo-bridge-v1/README.md",
        "experiments/exo-agent/extension/README.md",
        "experiments/exo-agent/bridge/README.md",
        "THIRD_PARTY_NOTICES.md",
        "crates/harness/src/exo/contract/descriptor.rs",
        "crates/harness/tests/support/exo_contract_projection.rs",
        "crates/harness/tests/support/exo_contract_schema_vectors.rs",
        "experiments/exo-agent/bridge/Cargo.toml",
        "experiments/exo-agent/bridge/Cargo.lock",
        "experiments/exo-agent/bridge/tests/process_oracle.rs",
        "protocol-artifact/exo-bridge-v1/fixtures/invalid-unknown.json",
        "protocol-artifact/exo-bridge-v1/golden/request.json",
        "protocol-artifact/exo-bridge-v1/golden/capability-source.json",
        "docs/evidence/exo-executor-process-oracle-20260915.md",
        "docs/evidence/exo-executor-process-oracle-20260915.json",
        "docs/decisions/0022-exo-lookup-duplex-bridge.md",
        "experiments/exo-agent/bridge/tests/lookup_oracle.rs",
        "crates/harness/tests/context_render_selected_limits.rs",
        "protocol-artifact/exo-bridge-v1/manifest.json",
    ] {
        assert!(
            pins.contains(&required),
            "required Exo pin location is not inventoried: {required}"
        );
    }
}

/// Every UTF-8 repository file that carries the reviewed candidate revision must be inventoried,
/// so a new pin cannot be added (or an existing pin renamed) without recording it in the manifest.
/// Generated and dependency trees are not published sources and are skipped.
#[test]
fn every_revision_bearing_file_is_inventoried() {
    const SKIPPED: [&str; 6] = [".git", ".vscode", "node_modules", "obj", "target", "vendor"];
    const MAXIMUM_BYTES: u64 = 4 * 1024 * 1024;
    let manifest: serde_json::Value =
        serde_json::from_str(exo_bridge_manifest()).expect("manifest is JSON");
    let pins = manifest["pin_locations"]
        .as_array()
        .expect("pin_locations is an array")
        .iter()
        .map(|value| value.as_str().expect("pin location is a string").to_owned())
        .collect::<Vec<_>>();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let mut pending = vec![root.clone()];
    let mut missing = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if SKIPPED.contains(&name.to_string_lossy().as_ref()) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file()
                || entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() > MAXIMUM_BYTES)
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            if std::str::from_utf8(&bytes).is_err()
                || !bytes
                    .windows(EXO_SOURCE_REVISION.len())
                    .any(|window| window == EXO_SOURCE_REVISION.as_bytes())
            {
                continue;
            }
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !pins.contains(&relative) {
                missing.push(relative);
            }
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "revision-bearing files absent from pin_locations: {missing:?}"
    );
}
