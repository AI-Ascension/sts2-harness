// SPDX-License-Identifier: MIT

use crate::{
    EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoIdentity, ExoToolCatalog, responses_capable,
    sha256_hex,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const MAX_EXECUTOR_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_EXTENSION_BYTES: usize = 64 * 1024;
pub const MAX_NODE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_CONFIGURATION_BYTES: usize = 32 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema: String,
    pub executor: PathBuf,
    pub executor_sha256: String,
    pub source_root: PathBuf,
    pub extension: PathBuf,
    pub extension_sha256: String,
    pub node: PathBuf,
    pub node_sha256: String,
    pub model: String,
    pub endpoint: String,
}

pub struct Loaded {
    pub config: Configuration,
    pub digest: String,
}

pub fn load(path: &str) -> Result<Loaded, &'static str> {
    load_profile(path, false)
}

pub fn load_profile(path: &str, lookup: bool) -> Result<Loaded, &'static str> {
    let bytes = read_bounded(Path::new(path), MAX_CONFIGURATION_BYTES)?;
    let config: Configuration = serde_json::from_slice(&bytes).map_err(|_| "exo_bridge_config")?;
    if config.schema
        != if lookup {
            "sts2.exo-lookup-config-v1"
        } else {
            "sts2.exo-one-shot-config-v1"
        }
        || !responses_capable(&config.model)
        || config.model.len() > 128
        || config.model.chars().any(char::is_control)
        || !config.source_root.is_absolute()
        || config.node.file_name().and_then(|name| name.to_str()) != Some("node")
    {
        return Err("exo_bridge_config");
    }
    verify_file(
        &config.executor,
        &config.executor_sha256,
        MAX_EXECUTOR_BYTES,
    )?;
    verify_file(
        &config.extension,
        &config.extension_sha256,
        MAX_EXTENSION_BYTES,
    )?;
    verify_file(&config.node, &config.node_sha256, MAX_NODE_BYTES)?;
    verify_source(&config.source_root)?;
    // This build owns exactly this extension, not an arbitrary operator-authored TypeScript agent.
    if config.extension_sha256
        != sha256_hex(if lookup {
            include_bytes!("../../../experiments/exo-agent/extension/src/lookup.ts").as_slice()
        } else {
            include_bytes!("../../../experiments/exo-agent/extension/src/index.ts").as_slice()
        })
    {
        return Err("exo_bridge_extension_identity");
    }
    let loaded = Loaded {
        config,
        digest: sha256_hex(bytes),
    };
    if loaded.validate_route(false).is_err() && loaded.validate_route(true).is_err() {
        return Err("exo_bridge_provider_route");
    }
    Ok(loaded)
}

impl Loaded {
    /// Binds the reviewed one-shot deployment to the same files that the bridge loads.
    ///
    /// The configuration loader has verified the executor, extension, Node and source identities.
    /// Re-read the effect-bearing artifacts here instead of trusting a second operator digest.
    /// The prompt is embedded in the reviewed extension, so its complete source binds that axis.
    /// `native_instance_id` comes from the gateway runtime configuration, not the Exo pin.
    pub fn inspected_identity(
        &self,
        bridge: &Path,
        native_instance_id: &str,
    ) -> Result<ExoIdentity, &'static str> {
        let package_digest = sha256_hex(read_bounded(&self.config.executor, MAX_EXECUTOR_BYTES)?);
        let extension_digest =
            sha256_hex(read_bounded(&self.config.extension, MAX_EXTENSION_BYTES)?);
        if package_digest != self.config.executor_sha256
            || extension_digest != self.config.extension_sha256
        {
            return Err("exo_bridge_package_identity");
        }
        Ok(ExoIdentity {
            source_revision: EXO_SOURCE_REVISION.to_owned(),
            package_digest: Some(package_digest),
            extension_digest: Some(extension_digest.clone()),
            bridge_digest: Some(sha256_hex(read_bounded(bridge, MAX_EXECUTOR_BYTES)?)),
            model_binding: Some(self.config.model.clone()),
            provider: Some("openai".to_owned()),
            endpoint: Some(self.config.endpoint.clone()),
            prompt_digest: Some(extension_digest),
            tool_digest: Some(ExoToolCatalog::reviewed().catalog_digest()),
            config_digest: Some(self.digest.clone()),
            contract_version: EXO_CONTRACT_VERSION.to_owned(),
            native_instance_id: Some(native_instance_id.to_owned()),
        })
    }

    pub fn lookup_description(&self) -> Result<Value, &'static str> {
        let mut value = self.description()?;
        value["schema"] = json!("sts2.exo-lookup-capability-v1");
        value["wire_version"] = json!(crate::exo_lookup_wire::EXO_LOOKUP_WIRE);
        value["tools"] = json!(["sts2_lookup_query", "sts2_lookup_read"]);
        value["tool_digest"] = json!(sha256_hex(b"sts2_lookup_query\nsts2_lookup_read\n"));
        value["decisions"] = json!(["action_id"]);
        value["max_tool_round_trips"] = json!(32);
        value["max_model_writes"] = json!(33);
        Ok(value)
    }
    pub fn validate_route(&self, synthetic: bool) -> Result<(), &'static str> {
        if synthetic {
            let port = self
                .config
                .endpoint
                .strip_prefix("http://127.0.0.1:")
                .and_then(|value| value.parse::<u16>().ok());
            if port.is_none_or(|port| port == 0) || self.config.model != "o3-pro" {
                return Err("exo_bridge_synthetic_route");
            }
        } else if self.config.endpoint != "https://api.openai.com/v1" {
            return Err("exo_bridge_provider_route");
        }
        Ok(())
    }

    pub fn description(&self) -> Result<Value, &'static str> {
        let executable = std::env::current_exe().map_err(|_| "exo_bridge_package")?;
        Ok(json!({
            "schema": "sts2.exo-one-shot-capability-v1",
            "source_revision": EXO_SOURCE_REVISION,
            "bridge_sha256": sha256_hex(read_bounded(&executable, MAX_EXECUTOR_BYTES)?),
            "executor_sha256": self.config.executor_sha256,
            "extension_sha256": self.config.extension_sha256,
            "node_sha256": self.config.node_sha256,
            "tool_digest": ExoToolCatalog::reviewed().catalog_digest(),
            "configuration_sha256": self.digest,
            "model": self.config.model,
            "endpoint": self.config.endpoint,
            "profiles": ["standard"],
            "context_modes": ["fresh"],
            "decisions": ["action", "plan", "wait", "reobserve"],
            "max_turns": 1,
            "max_tool_round_trips": 0,
            "model_calls": 0,
            "full_runtime_admission": false,
            "durable_recovery": "unverified",
            "native_game": "unverified"
        }))
    }
}

fn verify_file(path: &Path, expected: &str, maximum: usize) -> Result<(), &'static str> {
    if !path.is_absolute() || sha256_hex(read_bounded(path, maximum)?) != expected {
        return Err("exo_bridge_package_identity");
    }
    Ok(())
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "exo_bridge_unavailable")?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "exo_bridge_unavailable")?;
    if bytes.len() > maximum {
        return Err("exo_bridge_package_bound");
    }
    Ok(bytes)
}

fn verify_source(root: &Path) -> Result<(), &'static str> {
    for (args, expected) in [
        (vec!["rev-parse", "HEAD"], EXO_SOURCE_REVISION),
        (vec!["status", "--porcelain", "--untracked-files=no"], ""),
    ] {
        let output = Command::new("/usr/bin/git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| "exo_bridge_source")?;
        if !output.status.success()
            || std::str::from_utf8(&output.stdout).map(str::trim) != Ok(expected)
        {
            return Err("exo_bridge_source_identity");
        }
    }
    Ok(())
}
