// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use crate::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoProcessConfig, sha256_hex};

use super::{
    Configuration, Loaded, PrivateStateProfile, SyntheticInspectionError,
    SyntheticLoopbackInspection, canonical_regular_file, launch_arguments,
};

struct FixtureRoot(PathBuf);

impl FixtureRoot {
    fn new(name: &str) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "sts2-h391-synthetic-inspection-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct LoadedFixture {
    process: ExoProcessConfig,
    package: PathBuf,
    loaded: Loaded,
    digest: String,
}

fn loaded_fixture(root: &Path, endpoint: &str) -> Result<LoadedFixture, String> {
    let package = root.join("executor");
    let extension = root.join("extension.js");
    let node = root.join("node");
    let bridge = root.join("bridge");
    let configuration = root.join("config.json");
    std::fs::write(&package, b"synthetic executor").map_err(|error| error.to_string())?;
    std::fs::write(&extension, b"synthetic extension").map_err(|error| error.to_string())?;
    std::fs::write(&node, b"synthetic node").map_err(|error| error.to_string())?;
    std::fs::write(&bridge, b"synthetic bridge").map_err(|error| error.to_string())?;
    let config_bytes = br#"{"schema":"sts2.exo-one-shot-config-v2"}"#;
    std::fs::write(&configuration, config_bytes).map_err(|error| error.to_string())?;
    let digest = sha256_hex(config_bytes);
    let policy = crate::ExoPrivateStatePolicy {
        state_root: String::from("/opt/h391-fixture/state"),
        cache_root: String::from("/opt/h391-fixture/cache"),
        temp_root: String::from("/opt/h391-fixture/temp"),
        quota_bytes: 1 << 30,
        max_retention_days: 7,
        permissions_octal: 0o700,
    };
    let loaded = Loaded {
        config: Configuration {
            schema: String::from("sts2.exo-one-shot-config-v2"),
            executor_sha256: sha256_hex(std::fs::read(&package).map_err(|e| e.to_string())?),
            executor: package.clone(),
            source_root: root.join("source"),
            extension_sha256: sha256_hex(
                std::fs::read(&extension).map_err(|error| error.to_string())?,
            ),
            extension: extension.clone(),
            node_sha256: sha256_hex(std::fs::read(&node).map_err(|error| error.to_string())?),
            node,
            model: String::from("o3-pro"),
            endpoint: endpoint.to_owned(),
        },
        digest: digest.clone(),
        private_state: PrivateStateProfile::GuardedV2(policy),
    };
    let process = ExoProcessConfig::new(
        path_text(&bridge)?,
        vec![
            String::from("--synthetic"),
            path_text(&configuration)?,
            digest.clone(),
        ],
        None,
        Vec::new(),
    )
    .map_err(|_| String::from("fixture process configuration was refused"))?;
    Ok(LoadedFixture {
        process,
        package,
        loaded,
        digest,
    })
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| String::from("fixture path is not UTF-8"))
}

fn inspect_fixture(
    fixture: LoadedFixture,
) -> Result<SyntheticLoopbackInspection, SyntheticInspectionError> {
    let bridge = canonical_regular_file(Path::new(fixture.process.executable()))?;
    SyntheticLoopbackInspection::from_loaded(
        fixture.process,
        &fixture.package,
        "fixture-native-instance",
        &fixture.digest,
        &bridge,
        fixture.loaded,
    )
}

#[test]
fn launch_arguments_accept_only_the_exact_synthetic_config_and_digest_tuple() -> Result<(), String>
{
    let root = FixtureRoot::new("arguments")?;
    let config = root.path().join("config.json");
    let digest = "a".repeat(64);
    let valid = ExoProcessConfig::new(
        "/opt/bridge",
        vec![
            String::from("--synthetic"),
            path_text(&config)?,
            digest.clone(),
        ],
        None,
        Vec::new(),
    )
    .map_err(|_| String::from("valid argument fixture was refused"))?;
    assert_eq!(
        launch_arguments(&valid),
        Ok((config.as_path(), digest.as_str()))
    );

    for arguments in [
        vec![String::from("--run"), path_text(&config)?, digest.clone()],
        vec![
            String::from("--synthetic-v2"),
            path_text(&config)?,
            digest.clone(),
        ],
        vec![
            String::from("--synthetic"),
            path_text(&config)?,
            digest.clone(),
            String::from("extra"),
        ],
        vec![
            String::from("--synthetic"),
            String::from("relative.json"),
            digest.clone(),
        ],
        vec![
            String::from("--synthetic"),
            path_text(&config)?,
            String::from("bad"),
        ],
    ] {
        let process = ExoProcessConfig::new("/opt/bridge", arguments, None, Vec::new())
            .map_err(|_| String::from("negative argument fixture was refused too early"))?;
        assert_eq!(
            launch_arguments(&process),
            Err(SyntheticInspectionError::InvalidProcessConfiguration)
        );
    }
    Ok(())
}

#[test]
fn fake_loaded_structural_fixture_binds_digest_package_route_and_private_policy()
-> Result<(), String> {
    let root = FixtureRoot::new("loaded")?;
    let fixture = loaded_fixture(root.path(), "http://127.0.0.1:4319")?;
    let inspection = inspect_fixture(fixture)?;
    let (process, identity, _) = inspection.into_parts();
    assert_eq!(process.arguments()[0], "--synthetic");
    assert!(identity.is_complete());
    assert_eq!(identity.model_binding.as_deref(), Some("o3-pro"));
    assert_eq!(identity.endpoint.as_deref(), Some("http://127.0.0.1:4319"));
    assert_eq!(identity.source_revision, EXO_SOURCE_REVISION);
    assert_eq!(identity.contract_version, EXO_CONTRACT_VERSION);
    // This fake Loaded value tests the structural consumer only. It does not prove that inspect()
    // successfully loaded, source-verified or protected a real config file.
    Ok(())
}

#[test]
fn synthetic_inspection_rejects_digest_package_legacy_and_external_route_mismatches()
-> Result<(), String> {
    let root = FixtureRoot::new("mismatch")?;
    let mut bad_digest = loaded_fixture(root.path(), "http://127.0.0.1:4319")?;
    bad_digest.digest = "b".repeat(64);
    assert_eq!(
        inspect_fixture(bad_digest).err(),
        Some(SyntheticInspectionError::ConfigurationMismatch)
    );

    let mut bad_package = loaded_fixture(root.path(), "http://127.0.0.1:4319")?;
    let other = root.path().join("other-executor");
    std::fs::write(&other, b"different executor").map_err(|error| error.to_string())?;
    bad_package.package = other;
    assert_eq!(
        inspect_fixture(bad_package).err(),
        Some(SyntheticInspectionError::PackageMismatch)
    );

    let mut legacy = loaded_fixture(root.path(), "http://127.0.0.1:4319")?;
    legacy.loaded.private_state = PrivateStateProfile::LegacyV1;
    assert_eq!(
        inspect_fixture(legacy).err(),
        Some(SyntheticInspectionError::GuardedPrivateStateRequired)
    );

    let external = loaded_fixture(root.path(), "https://api.openai.com/v1")?;
    assert_eq!(
        inspect_fixture(external).err(),
        Some(SyntheticInspectionError::InvalidIdentity)
    );
    Ok(())
}
