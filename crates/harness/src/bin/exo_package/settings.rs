// SPDX-License-Identifier: MIT

use super::{Result, manifest::Manifest};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use sts2_harness::exo_bridge_configuration::{provider_route_admitted, synthetic_route_admitted};
use sts2_harness::{ExoPrivateStatePolicy, ExoRestrictedProfile, responses_capable};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProviderMode {
    Synthetic,
    Openai,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    schema: String,
    pub source_root: PathBuf,
    pub provider_mode: ProviderMode,
    pub model: String,
    pub endpoint: String,
    pub quota_bytes: u64,
    pub retention_days: u32,
}

pub(super) fn decode(bytes: &[u8]) -> Result<Settings> {
    let settings: Settings = serde_json::from_slice(bytes).map_err(|_| "exo_package_settings")?;
    if settings.schema != "sts2.exo-install-settings-v1"
        || !settings.source_root.is_absolute()
        || !responses_capable(&settings.model)
        || settings.model.len() > 128
        || settings.model.chars().any(char::is_control)
        || match settings.provider_mode {
            ProviderMode::Synthetic => {
                !synthetic_route_admitted(&settings.endpoint, &settings.model)
            }
            ProviderMode::Openai => !provider_route_admitted(&settings.endpoint),
        }
    {
        return Err("exo_package_settings");
    }
    // Check lexical bounds independently of whether the final install directory exists yet.
    settings.policy(Path::new("/var/lib/sts2-package-admission"))?;
    Ok(settings)
}

impl Settings {
    pub(super) fn policy(&self, installation: &Path) -> Result<ExoPrivateStatePolicy> {
        let mut policy = ExoRestrictedProfile::reviewed_private(installation.join("private")).state;
        policy.quota_bytes = self.quota_bytes;
        policy.max_retention_days = self.retention_days;
        policy
            .validate()
            .map_err(|_| "exo_package_private_policy")?;
        Ok(policy)
    }

    pub(super) fn configuration(
        &self,
        installation: &Path,
        manifest: &Manifest,
    ) -> Result<Vec<u8>> {
        let digest = |name: &str| {
            manifest
                .components
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.sha256.as_str())
                .ok_or("exo_package_component")
        };
        let artifacts = installation.join("artifacts");
        super::manifest::encode(&serde_json::json!({
            "schema": "sts2.exo-one-shot-config-v2",
            "executor": artifacts.join("sts2-exo-executor"),
            "executor_sha256": digest("sts2-exo-executor")?,
            "source_root": self.source_root,
            "extension": artifacts.join("index.ts"),
            "extension_sha256": digest("index.ts")?,
            "node": artifacts.join("node"),
            "node_sha256": digest("node")?,
            "model": self.model, "endpoint": self.endpoint,
            "private_state": self.policy(installation)?
        }))
    }
}
