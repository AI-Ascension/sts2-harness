// SPDX-License-Identifier: MIT

use super::{Result, filesystem as fs, manifest, settings};
use rustix::fs::{FlockOperation, Mode, OFlags, openat};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::Path;
use sts2_harness::sha256_hex;

const OWNER: &[u8] = b"sts2.exo-private-install-root-v1\n";
const MAX_INSTALLATIONS: usize = 64;

pub(super) struct Root {
    pub directory: File,
    _lock: File,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub schema: String,
    pub installation_id: String,
    pub manifest_sha256: String,
    pub settings_sha256: String,
    pub configuration_sha256: String,
}

pub(super) fn root(path: &Path, create: bool) -> Result<Root> {
    let directory = match fs::open_directory(path, true) {
        Ok(directory) => directory,
        Err(_) if create => {
            let parent = fs::open_directory(path.parent().ok_or("exo_package_path")?, false)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("exo_package_path")?;
            fs::create_directory(&parent, name)?
        }
        Err(error) => return Err(error),
    };
    let names = fs::names(&directory, MAX_INSTALLATIONS + 2)?;
    let new = create && names.is_empty();
    let lock = if new {
        fs::create_file(&directory, "install.lock")?
    } else {
        let file = File::from(
            openat(
                &directory,
                "install.lock",
                OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|_| "exo_package_install_root")?,
        );
        fs::check_file(&file, true)?;
        file
    };
    rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| "exo_package_install_busy")?;
    if new {
        fs::write_new(&directory, "owner", OWNER, false)?;
    }
    if fs::read(fs::open_file(&directory, "owner", true)?, 128)? != OWNER {
        return Err("exo_package_install_owner");
    }
    if fs::names(&directory, MAX_INSTALLATIONS + 2)?
        .iter()
        .any(|name| name != "owner" && name != "install.lock" && !manifest::hex(name, 64))
    {
        return Err("exo_package_foreign_install_entry");
    }
    Ok(Root {
        directory,
        _lock: lock,
    })
}

pub(super) fn install(
    input: &Path,
    manifest_path: &Path,
    manifest_digest: &str,
    settings_path: &Path,
    settings_digest: &str,
    destination: &Path,
) -> Result<String> {
    let manifest_bytes = trusted_bytes(manifest_path, manifest_digest, false)?;
    let package = manifest::decode(&manifest_bytes)?;
    let settings_bytes = trusted_bytes(settings_path, settings_digest, true)?;
    let configuration = settings::decode(&settings_bytes)?;
    if input.starts_with(destination)
        || destination.starts_with(input)
        || configuration.source_root.starts_with(destination)
        || destination.starts_with(&configuration.source_root)
    {
        return Err("exo_package_overlapping_roots");
    }
    let input = fs::open_directory(input, false)?;
    if manifest::inspect(&input)? != package.components {
        return Err("exo_package_input_changed");
    }
    let id = manifest::identity(&manifest_bytes, &settings_bytes);
    let installed_path = destination.join(&id);
    let config_bytes = configuration.configuration(&installed_path, &package)?;
    let root = root(destination, true)?;
    if fs::names(&root.directory, MAX_INSTALLATIONS + 2)?.len() >= MAX_INSTALLATIONS + 2 {
        return Err("exo_package_installation_bound");
    }
    let installed = fs::create_directory(&root.directory, &id)?;
    copy_artifacts(&input, &installed, &package)?;
    finish_install(
        &installed,
        &id,
        &manifest_bytes,
        &settings_bytes,
        &config_bytes,
    )?;
    root.directory.sync_all().map_err(|_| "exo_package_sync")?;
    Ok(id)
}

fn trusted_bytes(path: &Path, expected: &str, private: bool) -> Result<Vec<u8>> {
    if !manifest::hex(expected, 64) {
        return Err("exo_package_trusted_digest");
    }
    let bytes = fs::read_absolute(path, manifest::MAX_MANIFEST, private)?;
    if sha256_hex(&bytes) != expected {
        return Err("exo_package_trusted_digest");
    }
    Ok(bytes)
}

fn copy_artifacts(input: &File, installed: &File, package: &manifest::Manifest) -> Result<()> {
    let output = fs::create_directory(installed, "artifacts")?;
    for (component, policy) in package.components.iter().zip(manifest::INPUTS) {
        let mut source = fs::open_file(input, policy.name, false)?;
        fs::check_executable(&source, policy.executable)?;
        let mut target = fs::create_file(&output, policy.name)?;
        let (bytes, digest) = fs::transfer(&mut source, &mut target, policy.maximum)?;
        if bytes != component.bytes || digest != component.sha256 {
            return Err("exo_package_input_changed");
        }
        fs::seal(&target, policy.executable)?;
    }
    output.sync_all().map_err(|_| "exo_package_sync")
}

fn finish_install(
    installed: &File,
    id: &str,
    manifest: &[u8],
    settings: &[u8],
    config: &[u8],
) -> Result<()> {
    fs::write_new(installed, "manifest.json", manifest, false)?;
    fs::write_new(installed, "settings.json", settings, false)?;
    fs::write_new(installed, "config.json", config, false)?;
    let private = fs::create_directory(installed, "private")?;
    for name in ["state", "cache", "temp"] {
        fs::create_directory(&private, name)?;
    }
    let receipt = Receipt {
        schema: "sts2.exo-install-receipt-v1".to_owned(),
        installation_id: id.to_owned(),
        manifest_sha256: sha256_hex(manifest),
        settings_sha256: sha256_hex(settings),
        configuration_sha256: sha256_hex(config),
    };
    // Written last: interrupted installations stay visible but never pass inspection.
    fs::write_new(
        installed,
        "receipt.json",
        &manifest::encode(&receipt)?,
        false,
    )
}
