// SPDX-License-Identifier: MIT

use std::path::PathBuf;

pub fn pinned_exo_test_source() -> Result<PathBuf, String> {
    let source = std::env::var_os("STS2_EXO_TEST_SOURCE").ok_or_else(|| {
        String::from(
            "STS2_EXO_TEST_SOURCE must point to the clean checkout of the reviewed Exo revision",
        )
    })?;
    let source = PathBuf::from(source);
    if !source.is_absolute() {
        return Err(String::from(
            "STS2_EXO_TEST_SOURCE must be an absolute checkout path",
        ));
    }
    Ok(source)
}

/// Returns the Node binary used by the pinned Exo lifecycle fixtures.
///
/// CI keeps its existing `/usr/local/bin/node` default. Local validation can point at the
/// workflow-pinned Node release without changing global PATH or installing a system binary.
pub fn pinned_node_test_binary() -> Result<PathBuf, String> {
    let configured = std::env::var_os("STS2_EXO_TEST_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/local/bin/node"));
    if !configured.is_absolute() {
        return Err(String::from(
            "STS2_EXO_TEST_NODE must be an absolute executable path",
        ));
    }
    let node = configured
        .canonicalize()
        .map_err(|error| format!("configured Node test binary is unavailable: {error}"))?;
    let metadata = std::fs::metadata(&node)
        .map_err(|error| format!("configured Node test binary is unavailable: {error}"))?;
    if !metadata.is_file() {
        return Err(String::from(
            "configured Node test binary is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(String::from(
                "configured Node test binary is not executable",
            ));
        }
    }
    Ok(node)
}
