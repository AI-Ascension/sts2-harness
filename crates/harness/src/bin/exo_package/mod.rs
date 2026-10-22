// SPDX-License-Identifier: MIT

mod filesystem;
mod inspection;
mod install;
mod manifest;
mod settings;
mod tree;

#[cfg(test)]
mod tests;

use std::io::Write;
use std::path::Path;

type Result<T> = std::result::Result<T, &'static str>;

pub(super) fn execute(args: Vec<String>) -> Result<()> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["manifest", input, revision, output] => {
            let manifest = manifest::build(Path::new(input), revision)?;
            let bytes = manifest::encode(&manifest)?;
            filesystem::write_absolute_new(Path::new(output), &bytes)?;
            print(&serde_json::json!({
                "schema": "sts2.exo-package-result-v1",
                "operation": "manifest",
                "manifest_sha256": sts2_harness::sha256_hex(&bytes),
                "components": manifest.components.len(),
                "build_provenance": "operator_review_required",
                "inference_performed": false
            }))
        }
        [
            "install",
            input,
            manifest,
            manifest_digest,
            settings,
            settings_digest,
            root,
        ] => {
            let id = install::install(
                Path::new(input),
                Path::new(manifest),
                manifest_digest,
                Path::new(settings),
                settings_digest,
                Path::new(root),
            )?;
            print(&serde_json::json!({
                "schema": "sts2.exo-package-result-v1",
                "operation": "install",
                "installation_id": id,
                "state": "staged_requires_preflight_and_owner_readmission",
                "inference_performed": false
            }))
        }
        ["preflight", root, id] => print(&inspection::preflight(Path::new(root), id)?),
        ["rollback-check", root, from, to] => {
            let previous = inspection::preflight(Path::new(root), from)?;
            let target = inspection::preflight(Path::new(root), to)?;
            print(&serde_json::json!({
                "schema": "sts2.exo-package-result-v1",
                "operation": "rollback-check",
                "from": previous["installation_id"],
                "to": target["installation_id"],
                "state": "held_for_owner_readmission",
                "active_or_unknown_operations": "not_inspected_or_modified",
                "state_migration": "unsupported",
                "changes_applied": false,
                "inference_performed": false
            }))
        }
        _ => Err("exo_package_arguments"),
    }
}

fn print(value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| "exo_package_status")?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|()| stdout.write_all(b"\n"))
        .map_err(|_| "exo_package_status")
}
