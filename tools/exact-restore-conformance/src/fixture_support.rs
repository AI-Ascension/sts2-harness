// SPDX-License-Identifier: MIT

use super::*;
use sha2::{Digest, Sha256};

pub(crate) struct Paths {
    pub(crate) harness_root: PathBuf,
    pub(crate) harness_bin: PathBuf,
    pub(crate) mcp_bin: PathBuf,
    pub(crate) gateway_bin: PathBuf,
    pub(crate) mod_bin: PathBuf,
}

pub(crate) fn create_private_store(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).recursive(true);
        builder
            .create(path)
            .map_err(|error| format!("create exact store: {error}"))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("harden exact store permissions: {error}"))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path).map_err(|error| format!("create exact store: {error}"))?;
    Ok(())
}

pub(crate) fn run_case(paths: &Paths, outcome: &str, artifact_root: &Path) -> Result<(), String> {
    let root = artifact_root.join(outcome);
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|error| format!("clear case root: {error}"))?;
    }
    fs::create_dir_all(&root).map_err(|error| format!("create case root: {error}"))?;
    let result = super::run_case_inner(paths, outcome, &root);
    let status = if result.is_ok() { "passed" } else { "failed" };
    let _ = fs::write(
        root.join("result.json"),
        format!("{{\"outcome\":\"{outcome}\",\"status\":\"{status}\"}}\n"),
    );
    sanitize_logs(&root)?;
    if result.is_err() {
        eprintln!(
            "exact-restore case {outcome} retained at {}",
            root.display()
        );
    }
    result
}

fn sanitize_logs(root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|error| format!("read case evidence: {error}"))? {
        let path = entry
            .map_err(|error| format!("read case evidence entry: {error}"))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("log") {
            continue;
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("read evidence log {}: {error}", path.display()))?;
        let sanitized = contents
            .lines()
            .filter(|line| {
                let lower = line.to_ascii_lowercase();
                !lower.contains("authorization:")
                    && !lower.contains("bearer ")
                    && !lower.contains("secret")
                    && !lower.contains("token")
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{sanitized}\n"))
            .map_err(|error| format!("write sanitized evidence log: {error}"))?;
    }
    Ok(())
}

pub(crate) fn write_binary_provenance(paths: &Paths, artifact_root: &Path) -> Result<(), String> {
    let binaries = [
        ("harness", &paths.harness_bin),
        ("mcp", &paths.mcp_bin),
        ("gateway", &paths.gateway_bin),
        ("mod", &paths.mod_bin),
    ];
    let mut output =
        String::from("{\"schema\":\"sts2.exact-restore-binary-provenance.v1\",\"binaries\":{");
    for (index, (name, path)) in binaries.iter().enumerate() {
        let bytes = fs::read(path)
            .map_err(|error| format!("read {name} binary {}: {error}", path.display()))?;
        let digest = Sha256::digest(bytes);
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "\"{name}\":{{\"path\":\"{}\",\"sha256\":\"{}\"}}",
            name,
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
    }
    output.push_str("}}\n");
    fs::create_dir_all(artifact_root).map_err(|error| format!("create artifact root: {error}"))?;
    fs::write(artifact_root.join("binary-provenance.json"), output)
        .map_err(|error| format!("write binary provenance: {error}"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_store_is_0700_under_restrictive_umask() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("exact-store-test-{}", Uuid::new_v4()));
        create_private_store(&root)?;
        let mode = fs::metadata(&root)?.permissions().mode() & 0o777;
        fs::remove_dir_all(&root)?;
        assert_eq!(mode, 0o700);
        Ok(())
    }
}
