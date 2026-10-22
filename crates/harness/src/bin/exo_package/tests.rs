// SPDX-License-Identifier: MIT

mod boundary_tests;

use super::*;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    base: PathBuf,
    input: PathBuf,
    root: PathBuf,
    manifest: PathBuf,
    settings: PathBuf,
}

impl Fixture {
    fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let parent = PathBuf::from("/tmp/ai-ascension-exo-package-147-tests");
        match fs::DirBuilder::new().mode(0o700).create(&parent) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        filesystem::open_directory(&parent, true)?;
        let base = parent.join(uuid::Uuid::new_v4().simple().to_string());
        fs::DirBuilder::new().mode(0o700).create(&base)?;
        let fixture = Self {
            input: base.join("inputs"),
            root: base.join("installed"),
            manifest: base.join("manifest.json"),
            settings: base.join("settings.json"),
            base,
        };
        fs::DirBuilder::new().mode(0o700).create(&fixture.input)?;
        for input in manifest::INPUTS {
            let bytes = input.owned.unwrap_or(if input.executable {
                b"#!/bin/sh\nexit 77\n"
            } else {
                b"Original synthetic package-input fixture, MIT.\n"
            });
            fixture.write_input(input.name, bytes, input.executable)?;
        }
        fixture.write_settings(1024 * 1024)?;
        fixture.refresh_manifest()?;
        Ok(fixture)
    }

    fn write_input(&self, name: &str, bytes: &[u8], executable: bool) -> TestResult {
        fs::write(self.input.join(name), bytes)?;
        fs::set_permissions(
            self.input.join(name),
            fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
        )?;
        Ok(())
    }

    fn write_settings(&self, quota: u64) -> TestResult {
        let settings = serde_json::json!({
            "schema": "sts2.exo-install-settings-v1", "source_root": self.base.join("absent-exo"),
            "provider_mode": "synthetic", "model": "o3-pro", "endpoint": "http://127.0.0.1:9",
            "quota_bytes": quota, "retention_days": 1
        });
        fs::write(&self.settings, manifest::encode(&settings)?)?;
        fs::set_permissions(&self.settings, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    fn refresh_manifest(&self) -> TestResult {
        let value = manifest::build(&self.input, "0123456789abcdef0123456789abcdef01234567")?;
        fs::write(&self.manifest, manifest::encode(&value)?)?;
        fs::set_permissions(&self.manifest, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    fn install(&self) -> Result<String> {
        let manifest_bytes = fs::read(&self.manifest).map_err(|_| "fixture_manifest")?;
        let settings_bytes = fs::read(&self.settings).map_err(|_| "fixture_settings")?;
        install::install(
            &self.input,
            &self.manifest,
            &sts2_harness::sha256_hex(manifest_bytes),
            &self.settings,
            &sts2_harness::sha256_hex(settings_bytes),
            &self.root,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn manifest_is_reproducible_and_has_no_host_paths() -> TestResult {
    let fixture = Fixture::new()?;
    let bytes = fs::read(&fixture.manifest)?;
    let decoded = manifest::decode(&bytes)?;
    fixture.refresh_manifest()?;
    assert_eq!(bytes, fs::read(&fixture.manifest)?);
    let node = decoded
        .components
        .iter()
        .find(|entry| entry.name == "node")
        .ok_or("missing node")?;
    assert_eq!(node.bytes, b"#!/bin/sh\nexit 77\n".len() as u64);
    assert_eq!(
        node.sha256,
        sts2_harness::sha256_hex(b"#!/bin/sh\nexit 77\n")
    );
    assert!(!String::from_utf8(bytes)?.contains(fixture.base.to_str().ok_or("path")?));
    assert_eq!(decoded.components.len(), 15);
    Ok(())
}

#[test]
fn installs_exact_bytes_and_private_configuration_without_running_inputs() -> TestResult {
    let fixture = Fixture::new()?;
    let marker = fixture.base.join("unexpected-execution");
    let script = format!(
        "#!/bin/sh\nprintf executed > '{}'\nexit 1\n",
        marker.display()
    );
    fixture.write_input("sts2-exo-bridge", script.as_bytes(), true)?;
    fixture.refresh_manifest()?;
    let id = fixture.install()?;
    let verified = inspection::verify(&fixture.root, &id)?;
    for input in manifest::INPUTS {
        assert_eq!(
            fs::read(fixture.input.join(input.name))?,
            fs::read(verified.path.join("artifacts").join(input.name))?
        );
        let mode = fs::metadata(verified.path.join("artifacts").join(input.name))?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, if input.executable { 0o500 } else { 0o400 });
    }
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(verified.path.join("config.json"))?)?;
    assert_eq!(config["schema"], "sts2.exo-one-shot-config-v2");
    assert_eq!(
        config["executor"],
        serde_json::json!(verified.path.join("artifacts/sts2-exo-executor"))
    );
    assert!(!marker.exists());
    assert!(inspection::preflight(&fixture.root, &id).is_err());
    assert!(
        !marker.exists(),
        "missing actual Exo source must fail before any probe executable"
    );
    Ok(())
}

#[test]
fn upgrade_and_failed_rollback_leave_unknown_operations_and_old_pins_unchanged() -> TestResult {
    let fixture = Fixture::new()?;
    let original = fixture.install()?;
    let old = fixture.root.join(&original);
    let configuration = fs::read(old.join("config.json"))?;
    let journal = old.join("private/state/owner-journal");
    let unknown = b"Original synthetic opaque encrypted-journal stand-in: UNKNOWN request-1";
    fs::write(&journal, unknown)?;
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o600))?;
    assert!(
        fixture.install().is_err(),
        "in-place replacement is forbidden"
    );
    fixture.write_input("sts2-exo-bridge", b"#!/bin/sh\nexit 78\n", true)?;
    fixture.refresh_manifest()?;
    let next = fixture.install()?;
    assert_ne!(original, next);
    assert_eq!(fs::read(&journal)?, unknown);
    assert_eq!(fs::read(old.join("config.json"))?, configuration);
    assert!(inspection::verify(&fixture.root, &original).is_ok());
    assert!(inspection::verify(&fixture.root, &next).is_ok());
    let args = vec![
        "rollback-check".to_owned(),
        fixture.root.to_string_lossy().into_owned(),
        original,
        next,
    ];
    assert!(
        execute(args).is_err(),
        "unverified Exo source cannot become a successful rollback check"
    );
    assert_eq!(fs::read(&journal)?, unknown);
    assert_eq!(fs::read(old.join("config.json"))?, configuration);
    assert!(!fixture.root.join("current").exists());
    Ok(())
}

#[test]
fn model_configuration_change_creates_a_new_installation_identity() -> TestResult {
    let fixture = Fixture::new()?;
    let original = fixture.install()?;
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&fixture.settings)?)?;
    value["endpoint"] = serde_json::json!("http://127.0.0.1:10");
    fs::write(&fixture.settings, manifest::encode(&value)?)?;
    let successor = fixture.install()?;
    assert_ne!(original, successor);
    assert_eq!(
        inspection::verify(&fixture.root, &original)?
            .settings
            .endpoint,
        "http://127.0.0.1:9"
    );
    assert_eq!(
        inspection::verify(&fixture.root, &successor)?
            .settings
            .endpoint,
        "http://127.0.0.1:10"
    );
    Ok(())
}

#[test]
fn preflight_rejects_changed_artifacts_config_and_partial_installations() -> TestResult {
    let fixture = Fixture::new()?;
    let id = fixture.install()?;
    let installed = fixture.root.join(&id);
    let artifact = installed.join("artifacts/sts2-exo-executor");
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o700))?;
    fs::write(&artifact, b"changed")?;
    assert!(inspection::verify(&fixture.root, &id).is_err());
    fs::write(
        &artifact,
        fs::read(fixture.input.join("sts2-exo-executor"))?,
    )?;
    assert!(inspection::verify(&fixture.root, &id).is_ok());
    fs::set_permissions(
        installed.join("config.json"),
        fs::Permissions::from_mode(0o600),
    )?;
    fs::write(installed.join("config.json"), b"{}")?;
    assert!(inspection::verify(&fixture.root, &id).is_err());
    fs::remove_file(installed.join("receipt.json"))?;
    assert!(inspection::verify(&fixture.root, &id).is_err());
    Ok(())
}

#[test]
fn quota_observation_refuses_without_removing_unknown_private_bytes() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write_settings(1)?;
    let id = fixture.install()?;
    let unknown = fixture.root.join(&id).join("private/state/unknown");
    fs::write(&unknown, b"unknown")?;
    fs::set_permissions(&unknown, fs::Permissions::from_mode(0o600))?;
    assert!(matches!(
        inspection::verify(&fixture.root, &id),
        Err("exo_package_private_quota")
    ));
    assert_eq!(fs::read(unknown)?, b"unknown");
    Ok(())
}
