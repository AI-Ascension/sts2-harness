// SPDX-License-Identifier: MIT

//! Offline file-identity checks; no Exo, model, gateway or game process is launched.

use std::path::PathBuf;
use sts2_harness::exo_bridge_configuration::{Configuration, Loaded};
use sts2_harness::{ExoToolCatalog, sha256_hex};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/exo-inspection-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn loaded(&self) -> Result<Loaded, Box<dyn std::error::Error>> {
        let executor = self.0.join("executor");
        let extension = self.0.join("extension.ts");
        std::fs::write(&executor, b"synthetic executor")?;
        std::fs::write(&extension, b"synthetic extension and prompt")?;
        std::fs::write(self.0.join("bridge"), b"synthetic bridge")?;
        Ok(Loaded {
            config: Configuration {
                schema: "sts2.exo-one-shot-config-v1".to_owned(),
                executor,
                executor_sha256: sha256_hex(b"synthetic executor"),
                source_root: self.0.clone(),
                extension,
                extension_sha256: sha256_hex(b"synthetic extension and prompt"),
                node: self.0.join("node"),
                node_sha256: sha256_hex(b"synthetic node"),
                model: "o3-pro".to_owned(),
                endpoint: "https://api.openai.com/v1".to_owned(),
            },
            digest: sha256_hex(b"synthetic configuration"),
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn inspection_uses_launch_artifacts_and_gateway_instance_not_operator_pins()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let loaded = fixture.loaded()?;
    let identity = loaded.inspected_identity(&fixture.0.join("bridge"), "instance:gateway")?;
    assert_eq!(
        identity.package_digest,
        Some(sha256_hex(b"synthetic executor"))
    );
    assert_eq!(
        identity.extension_digest,
        Some(sha256_hex(b"synthetic extension and prompt"))
    );
    assert_eq!(identity.prompt_digest, identity.extension_digest);
    assert_eq!(
        identity.bridge_digest,
        Some(sha256_hex(b"synthetic bridge"))
    );
    assert_eq!(identity.config_digest, Some(loaded.digest));
    assert_eq!(
        identity.tool_digest,
        Some(ExoToolCatalog::reviewed().catalog_digest())
    );
    assert_eq!(
        identity.native_instance_id.as_deref(),
        Some("instance:gateway")
    );
    assert_eq!(identity.model_binding.as_deref(), Some("o3-pro"));
    assert_eq!(identity.provider.as_deref(), Some("openai"));
    Ok(())
}

#[test]
fn changed_executor_or_extension_is_refused_after_configuration_load()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    for target in ["executor", "extension.ts"] {
        let loaded = fixture.loaded()?;
        std::fs::write(fixture.0.join(target), b"swapped bytes")?;
        assert_eq!(
            loaded.inspected_identity(&fixture.0.join("bridge"), "instance:gateway"),
            Err("exo_bridge_package_identity")
        );
    }
    Ok(())
}
