// SPDX-License-Identifier: MIT

use super::super::types::MAX_FRAME_BYTES;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_BYTES: usize = 2048;
const MAX_ENVIRONMENT_NAMES: usize = 32;
const MAX_ENVIRONMENT_NAME_BYTES: usize = 128;
const ISOLATED_ENVIRONMENT_NAMES: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "CODEX_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "TMPDIR",
    "TMP",
    "TEMP",
];
const APPROVED_INHERITED_ENVIRONMENT_NAMES: &[&str] = &["OPENAI_API_KEY"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NativeProcessConfig {
    pub(super) executable: String,
    arguments: Vec<String>,
    pub(super) working_directory: PathBuf,
    inherited_environment: Vec<String>,
    pub(super) state_root: PathBuf,
    pub(super) state_quota_bytes: u64,
    pub(super) max_frame_bytes: usize,
}

impl NativeProcessConfig {
    pub(super) fn new(
        executable: impl Into<String>,
        arguments: Vec<String>,
        working_directory: impl Into<PathBuf>,
        inherited_environment: Vec<String>,
        state_root: impl Into<PathBuf>,
    ) -> Result<Self, NativeProcessConfigError> {
        let value = Self {
            executable: executable.into(),
            arguments,
            working_directory: working_directory.into(),
            inherited_environment,
            state_root: state_root.into(),
            state_quota_bytes: super::MAX_NATIVE_STATE_BYTES,
            max_frame_bytes: MAX_FRAME_BYTES,
        };
        if !valid_executable(&value.executable)
            || value.arguments.len() > MAX_ARGUMENTS
            || value
                .arguments
                .iter()
                .any(|v| !valid_path(v, MAX_ARGUMENT_BYTES))
            || !valid_root_path(&value.working_directory)
            || !valid_root_path(&value.state_root)
            || value.inherited_environment.len() > MAX_ENVIRONMENT_NAMES
            || !valid_environment_names(&value.inherited_environment)
        {
            return Err(NativeProcessConfigError::Invalid);
        }
        Ok(value)
    }

    #[must_use]
    pub(super) fn executable(&self) -> &str {
        &self.executable
    }

    #[must_use]
    pub(super) fn arguments(&self) -> &[String] {
        &self.arguments
    }

    #[must_use]
    pub(super) fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    #[must_use]
    pub(super) fn state_root(&self) -> &Path {
        &self.state_root
    }

    #[must_use]
    pub(super) fn inherited_environment(&self) -> &[String] {
        &self.inherited_environment
    }

    /// Clear the parent environment and bind every conventional home/config/cache/temp root to
    /// the broker-owned state directory before adding explicitly approved secret names.
    pub(super) fn apply_isolated_environment(&self, command: &mut Command) {
        command.env_clear();
        for name in ISOLATED_ENVIRONMENT_NAMES {
            command.env(name, &self.state_root);
        }
        for name in self.inherited_environment() {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NativeProcessConfigError {
    Invalid,
}

impl std::fmt::Display for NativeProcessConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native process configuration is invalid")
    }
}

impl std::error::Error for NativeProcessConfigError {}

fn valid_path(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
        && !value.contains('\0')
}

fn valid_executable(value: &str) -> bool {
    valid_path(value, 1024) && Path::new(value).is_absolute()
}

fn valid_root_path(path: &Path) -> bool {
    path.is_absolute()
        && !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| !matches!(component, Component::ParentDir))
}

fn valid_environment_names(names: &[String]) -> bool {
    let mut unique = BTreeSet::new();
    names.iter().all(|name| {
        !name.is_empty()
            && name.len() <= MAX_ENVIRONMENT_NAME_BYTES
            && name.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_alphabetic() || byte == b'_'
                } else {
                    byte.is_ascii_alphanumeric() || byte == b'_'
                }
            })
            && !matches!(
                name.as_str(),
                "LD_PRELOAD"
                    | "LD_LIBRARY_PATH"
                    | "DYLD_INSERT_LIBRARIES"
                    | "HOME"
                    | "USERPROFILE"
                    | "APPDATA"
                    | "LOCALAPPDATA"
                    | "CODEX_HOME"
                    | "XDG_CONFIG_HOME"
                    | "XDG_DATA_HOME"
                    | "XDG_CACHE_HOME"
                    | "XDG_RUNTIME_DIR"
                    | "TMPDIR"
                    | "TMP"
                    | "TEMP"
            )
            && APPROVED_INHERITED_ENVIRONMENT_NAMES.contains(&name.as_str())
            && unique.insert(name)
    })
}

#[cfg(test)]
mod tests {
    use super::{NativeProcessConfig, valid_environment_names, valid_executable, valid_root_path};
    use std::ffi::{OsStr, OsString};
    use std::process::Command;

    #[test]
    fn process_paths_require_absolute_roots_and_executable() {
        assert!(valid_executable("/opt/codex/bin/codex"));
        assert!(!valid_executable("codex"));
        assert!(!valid_executable("../codex"));
        assert!(!valid_environment_names(&["HOME".to_owned()]));
        assert!(!valid_environment_names(&["CODEX_HOME".to_owned()]));
        assert!(valid_environment_names(&["OPENAI_API_KEY".to_owned()]));
        assert!(!valid_environment_names(&["PATH".to_owned()]));
        assert!(!valid_environment_names(&["AWS_ACCESS_KEY_ID".to_owned()]));
        assert!(valid_root_path(std::path::Path::new("/tmp/provider-root")));
        assert!(!valid_root_path(std::path::Path::new("relative-root")));
        assert!(!valid_root_path(std::path::Path::new("/tmp/../escape")));
    }

    #[test]
    fn process_environment_is_bound_to_state_root() {
        let config = NativeProcessConfig::new(
            "/opt/codex/bin/codex",
            Vec::new(),
            "/tmp/provider-work",
            Vec::new(),
            "/tmp/provider-state",
        );
        assert!(config.is_ok());
        let Some(config) = config.ok() else {
            return;
        };
        let mut command = Command::new("/bin/true");
        config.apply_isolated_environment(&mut command);
        let value = |name: &str| {
            command
                .get_envs()
                .find_map(|(key, value)| {
                    (key == OsStr::new(name)).then(|| value.map(OsString::from))
                })
                .flatten()
        };
        for name in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "CODEX_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
            "TMP",
            "TEMP",
        ] {
            assert_eq!(value(name), Some(OsString::from("/tmp/provider-state")));
        }
        assert!(value("PATH").is_none());
    }
}
