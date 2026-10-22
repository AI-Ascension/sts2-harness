// SPDX-License-Identifier: MIT
//! Actual package CLI with original synthetic executable/license bytes. No Exo or provider runs.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use sts2_harness::sha256_hex;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SYNTHETIC_SECRET: &str = "synthetic-package-test-secret-must-not-be-printed";

struct Installation(PathBuf);

impl Installation {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let parent = PathBuf::from("/tmp/ai-ascension-exo-package-147-tests");
        match fs::DirBuilder::new().mode(0o700).create(&parent) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        if fs::symlink_metadata(&parent)?.file_type().is_symlink()
            || fs::metadata(&parent)?.permissions().mode() & 0o7777 != 0o700
        {
            return Err("unsafe fixture parent".into());
        }
        let root = parent.join(uuid::Uuid::new_v4().simple().to_string());
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        Ok(Self(root))
    }

    fn stage(&self) -> TestResult {
        let input = self.0.join("inputs");
        fs::DirBuilder::new().mode(0o700).create(&input)?;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (source, destination) in [
            ("LICENSE", "LICENSE.harness"),
            ("THIRD_PARTY_NOTICES.md", "THIRD_PARTY_NOTICES.harness"),
            ("Cargo.lock", "harness.Cargo.lock"),
            ("rust-toolchain.toml", "rust-toolchain.toml"),
            (
                "experiments/exo-agent/bridge/Cargo.lock",
                "executor.Cargo.lock",
            ),
            (
                "experiments/exo-agent/extension/package.json",
                "extension.package.json",
            ),
            ("experiments/exo-agent/extension/src/index.ts", "index.ts"),
        ] {
            self.write(
                &input.join(destination),
                &fs::read(root.join(source))?,
                false,
            )?;
        }
        for name in [
            "LICENSE.exo",
            "LICENSE.node",
            "exo.Cargo.lock",
            "exo.pnpm-lock.yaml",
        ] {
            self.write(
                &input.join(name),
                b"Original synthetic fixture, MIT; not an upstream artifact.\n",
                false,
            )?;
        }
        let marker = self.0.join("probe-executed");
        let script = format!(
            "#!/bin/sh\nprintf forbidden > '{}'\nexit 90\n",
            marker.display()
        );
        for name in [
            "node",
            "sts2-exo-bridge",
            "sts2-exo-executor",
            "sts2-exo-package",
        ] {
            self.write(&input.join(name), script.as_bytes(), true)?;
        }
        let settings = serde_json::json!({
            "schema": "sts2.exo-install-settings-v1", "source_root": self.0.join("unavailable-source"),
            "provider_mode": "synthetic", "model": "o3-pro", "endpoint": "http://127.0.0.1:9",
            "quota_bytes": 1048576, "retention_days": 1
        });
        self.write(
            &self.0.join("settings.json"),
            &serde_json::to_vec(&settings)?,
            false,
        )
    }

    fn write(&self, path: &Path, bytes: &[u8], executable: bool) -> TestResult {
        fs::write(path, bytes)?;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
        )?;
        Ok(())
    }
}

impl Drop for Installation {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn command(args: &[String]) -> Result<Output, Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_sts2-exo-package"))
        .args(args)
        .env_clear()
        .env("STS2_EXO_MODEL_KEY", SYNTHETIC_SECRET)
        .output()?;
    assert!(!String::from_utf8_lossy(&output.stdout).contains(SYNTHETIC_SECRET));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SYNTHETIC_SECRET));
    Ok(output)
}

#[test]
fn compiled_cli_installs_pinned_bytes_and_refuses_missing_source_without_execution() -> TestResult {
    let fixture = Installation::new()?;
    fixture.stage()?;
    let path = |name: &str| fixture.0.join(name).to_string_lossy().into_owned();
    let result = command(&[
        "manifest".to_owned(),
        path("inputs"),
        "a".repeat(40),
        path("manifest.json"),
    ])?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = fs::read(fixture.0.join("manifest.json"))?;
    let status: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(status["manifest_sha256"], sha256_hex(&manifest));
    let settings = fs::read(fixture.0.join("settings.json"))?;
    let args = [
        "install".to_owned(),
        path("inputs"),
        path("manifest.json"),
        sha256_hex(&manifest),
        path("settings.json"),
        sha256_hex(&settings),
        path("installed"),
    ];
    let result = command(&args)?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    let id = status["installation_id"]
        .as_str()
        .ok_or("installation id")?;
    assert_eq!(id.len(), 64);
    let installed = fixture.0.join("installed").join(id);
    assert_eq!(
        fs::read(installed.join("artifacts/index.ts"))?,
        fs::read(fixture.0.join("inputs/index.ts"))?
    );
    let result = command(&["preflight".to_owned(), path("installed"), id.to_owned()])?;
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(result.stderr, b"exo_package_path\n");
    assert!(!fixture.0.join("probe-executed").exists());
    assert!(
        !command(&args)?.status.success(),
        "same installation cannot be replaced"
    );
    assert_eq!(fs::read(installed.join("manifest.json"))?, manifest);
    Ok(())
}

#[test]
fn unknown_command_never_echoes_private_arguments() -> TestResult {
    let result = command(&["activate".to_owned(), SYNTHETIC_SECRET.to_owned()])?;
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(result.stderr, b"exo_package_arguments\n");
    Ok(())
}
