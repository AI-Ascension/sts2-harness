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
    ] {
        assert!(
            pins.contains(&required),
            "required Exo pin location is not inventoried: {required}"
        );
    }
}
