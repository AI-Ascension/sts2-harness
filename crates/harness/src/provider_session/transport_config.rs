// SPDX-License-Identifier: MIT

use super::super::types::MAX_FRAME_BYTES;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_BYTES: usize = 2048;
const MAX_ENVIRONMENT_NAMES: usize = 32;
const MAX_ENVIRONMENT_NAME_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NativeProcessConfig {
    pub(super) executable: String,
    arguments: Vec<String>,
    pub(super) working_directory: PathBuf,
    inherited_environment: Vec<String>,
    pub(super) state_root: PathBuf,
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
            max_frame_bytes: MAX_FRAME_BYTES,
        };
        if !valid_path(&value.executable, 1024)
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
                "LD_PRELOAD" | "LD_LIBRARY_PATH" | "DYLD_INSERT_LIBRARIES"
            )
            && unique.insert(name)
    })
}
