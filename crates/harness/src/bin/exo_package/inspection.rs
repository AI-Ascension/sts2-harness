// SPDX-License-Identifier: MIT

use super::{Result, filesystem as fs, install, manifest, settings, tree};

#[cfg(test)]
mod probe_tests;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use sts2_harness::{exo_bridge_configuration, sha256_hex};
use tokio::io::AsyncReadExt;

pub(super) struct Verified {
    pub path: PathBuf,
    pub package: manifest::Manifest,
    pub settings: settings::Settings,
    pub config_digest: String,
    pub manifest_digest: String,
    pub private_bytes: u64,
}

pub(super) fn verify(root: &Path, id: &str) -> Result<Verified> {
    if !manifest::hex(id, 64) {
        return Err("exo_package_installation_id");
    }
    let locked = install::root(root, false)?;
    let directory = fs::child_directory(&locked.directory, id)?;
    if fs::names(&directory, 6)?
        != [
            "artifacts",
            "config.json",
            "manifest.json",
            "private",
            "receipt.json",
            "settings.json",
        ]
    {
        return Err("exo_package_installation_inventory");
    }
    let read = |name| {
        fs::read(
            fs::open_file(&directory, name, true)?,
            manifest::MAX_MANIFEST,
        )
    };
    let manifest_bytes = read("manifest.json")?;
    let settings_bytes = read("settings.json")?;
    let config_bytes = read("config.json")?;
    let receipt: install::Receipt =
        serde_json::from_slice(&read("receipt.json")?).map_err(|_| "exo_package_receipt")?;
    let path = root.join(id);
    let package = manifest::decode(&manifest_bytes)?;
    let settings = settings::decode(&settings_bytes)?;
    if receipt.schema != "sts2.exo-install-receipt-v1"
        || receipt.installation_id != id
        || manifest::identity(&manifest_bytes, &settings_bytes) != id
        || receipt.manifest_sha256 != sha256_hex(&manifest_bytes)
        || receipt.settings_sha256 != sha256_hex(&settings_bytes)
        || receipt.configuration_sha256 != sha256_hex(&config_bytes)
        || settings.configuration(&path, &package)? != config_bytes
    {
        return Err("exo_package_installation_identity");
    }
    let artifacts = fs::child_directory(&directory, "artifacts")?;
    if manifest::inspect(&artifacts)? != package.components {
        return Err("exo_package_artifact_changed");
    }
    let private = fs::child_directory(&directory, "private")?;
    let private_bytes = tree::private_usage(&private, settings.quota_bytes)?;
    Ok(Verified {
        path,
        package,
        settings,
        private_bytes,
        config_digest: receipt.configuration_sha256,
        manifest_digest: receipt.manifest_sha256,
    })
}

pub(super) fn preflight(root: &Path, id: &str) -> Result<serde_json::Value> {
    let verified = verify(root, id)?;
    // The package lock has been released. Installations are append-only; this never holds a
    // filesystem mutation lock over source inspection or a subprocess boundary.
    verify_source_inputs(&verified)?;
    let config_path = verified.path.join("config.json");
    let config_path = config_path.to_str().ok_or("exo_package_path")?;
    let loaded = exo_bridge_configuration::load(config_path)
        .map_err(|_| "exo_package_bridge_source_or_configuration")?;
    if loaded.digest != verified.config_digest || loaded.guarded_private_state().is_none() {
        return Err("exo_package_bridge_configuration_identity");
    }
    let artifacts = verified.path.join("artifacts");
    if invoke(&artifacts.join("node"), &["--version"], &verified.path, 64)? != b"v22.14.0\n" {
        return Err("exo_package_node_version");
    }
    let mut expected = loaded
        .description()
        .map_err(|_| "exo_package_description")?;
    expected["bridge_sha256"] = serde_json::json!(component_digest(&verified, "sts2-exo-bridge")?);
    let observed = invoke(
        &artifacts.join("sts2-exo-bridge"),
        &["--describe", config_path],
        &verified.path,
        manifest::MAX_MANIFEST,
    )?;
    if serde_json::to_vec(&expected).map_err(|_| "exo_package_description")? != observed {
        return Err("exo_package_bridge_description_identity");
    }
    Ok(serde_json::json!({
        "schema": "sts2.exo-package-result-v1", "operation": "preflight",
        "installation_id": id, "manifest_sha256": verified.manifest_digest,
        "configuration_sha256": verified.config_digest,
        "state": "package_preflight_passed_owner_readmission_required",
        "source_revision": verified.package.exo_revision, "node": verified.package.node,
        "profile": "standard", "context": "fresh", "private_bytes_observed": verified.private_bytes,
        "private_quota_bytes": verified.settings.quota_bytes,
        "model_authentication": "unverified", "downstream_readiness": "unverified",
        "executor_loadability": "unverified", "synthetic_smoke": "unverified",
        "full_runtime_admission": false, "inference_performed": false,
        "active_or_unknown_operations": "not_inspected_or_modified"
    }))
}

fn component_digest<'a>(verified: &'a Verified, name: &str) -> Result<&'a str> {
    verified
        .package
        .components
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.sha256.as_str())
        .ok_or("exo_package_component")
}

fn verify_source_inputs(verified: &Verified) -> Result<()> {
    let source = fs::open_directory(&verified.settings.source_root, false)?;
    for (source_name, installed_name) in [
        ("Cargo.lock", "exo.Cargo.lock"),
        ("pnpm-lock.yaml", "exo.pnpm-lock.yaml"),
    ] {
        let (bytes, digest) =
            fs::hash(fs::open_file(&source, source_name, false)?, 8 * 1024 * 1024)?;
        if bytes == 0 || digest != component_digest(verified, installed_name)? {
            return Err("exo_package_source_input_identity");
        }
    }
    Ok(())
}

fn invoke(executable: &Path, args: &[&str], directory: &Path, maximum: usize) -> Result<Vec<u8>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "exo_package_probe")?;
    runtime.block_on(invoke_bounded(
        executable,
        args,
        directory,
        maximum,
        Duration::from_secs(10),
    ))
}

async fn invoke_bounded(
    executable: &Path,
    args: &[&str],
    directory: &Path,
    maximum: usize,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut child = tokio::process::Command::new(executable)
        .args(args)
        .current_dir(directory)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "exo_package_probe_start")?;
    let stdout = child.stdout.take().ok_or("exo_package_probe")?;
    let result = tokio::time::timeout(timeout, async {
        let mut bytes = Vec::new();
        stdout
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "exo_package_probe_output")?;
        if bytes.len() > maximum {
            return Err("exo_package_probe_output_bound");
        }
        if !child
            .wait()
            .await
            .map_err(|_| "exo_package_probe_exit")?
            .success()
        {
            return Err("exo_package_probe_failed");
        }
        Ok(bytes)
    })
    .await;
    match result {
        Ok(Ok(bytes)) => Ok(bytes),
        failed => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_millis(250), child.wait()).await;
            failed.unwrap_or(Err("exo_package_probe_timeout"))
        }
    }
}
