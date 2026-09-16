// SPDX-License-Identifier: MIT

/// Resolves the content-addressed artifact directory used by durable branches.
pub(crate) fn artifact_store_path() -> Result<PathBuf, String> {
    match std::env::var("STS2_EXACT_ARTIFACT_STORE_PATH") {
        Ok(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        Ok(_) => Err(String::from(
            "STS2_EXACT_ARTIFACT_STORE_PATH must not be empty",
        )),
        Err(std::env::VarError::NotPresent) => {
            let execution = std::env::var("STS2_EXECUTION_STORE_PATH")
                .unwrap_or_else(|_| String::from("harness-execution.sqlite3"));
            Ok(PathBuf::from(execution).with_file_name("harness-exact-artifacts"))
        }
        Err(std::env::VarError::NotUnicode(_)) => Err(String::from(
            "STS2_EXACT_ARTIFACT_STORE_PATH is not valid UTF-8",
        )),
    }
}
