// SPDX-License-Identifier: MIT

use super::*;

pub(crate) struct Paths {
    pub(crate) harness_root: PathBuf,
    pub(crate) harness_bin: PathBuf,
    pub(crate) mcp_bin: PathBuf,
    pub(crate) gateway_bin: PathBuf,
    pub(crate) mod_bin: PathBuf,
}

pub(crate) fn run_case(paths: &Paths, outcome: &str) -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("sts2-exact-conformance-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).map_err(|error| format!("create case root: {error}"))?;
    let result = super::run_case_inner(paths, outcome, &root);
    if result.is_err() {
        eprintln!(
            "exact-restore case {outcome} retained at {}",
            root.display()
        );
    } else {
        let _ = fs::remove_dir_all(&root);
    }
    result
}
