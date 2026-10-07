// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use crate::{EXO_SOURCE_REVISION, ExoIdentity, ExoPrivateStatePolicy, ExoProcessConfig};

use super::{Loaded, PrivateStateProfile, load};

/// Opaque proof produced by bounded file inspection; retains the exact launch configuration.
/// Paths are re-read at launch, so this does not prevent local file replacement between inspection
/// and use.
pub struct SyntheticLoopbackInspection {
    process: ExoProcessConfig,
    identity: ExoIdentity,
    private_state: ExoPrivateStatePolicy,
}

impl SyntheticLoopbackInspection {
    /// Inspects exact `--synthetic CONFIG DIGEST` arguments without starting the bridge.
    /// The package locator must match the guarded executor; the native-instance id is caller-supplied
    /// metadata, not an independent attestation. Returns path-free errors when outside this profile.
    pub fn inspect(
        process: ExoProcessConfig,
        package_path: impl AsRef<Path>,
        native_instance_id: &str,
    ) -> Result<Self, SyntheticInspectionError> {
        let (configuration_path, expected_digest) = launch_arguments(&process)?;
        let expected_digest = expected_digest.to_owned();
        if !process.inherited_environment().is_empty() {
            return Err(SyntheticInspectionError::InvalidProcessConfiguration);
        }
        let configuration_path = canonical_regular_file(configuration_path)?;
        let bridge_path = canonical_regular_file(Path::new(process.executable()))?;
        let loaded = load(
            configuration_path
                .to_str()
                .ok_or(SyntheticInspectionError::InvalidProcessConfiguration)?,
        )
        .map_err(|_| SyntheticInspectionError::ConfigurationUnavailable)?;
        loaded
            .validate_route(true)
            .map_err(|_| SyntheticInspectionError::InvalidIdentity)?;
        Self::from_loaded(
            process,
            package_path.as_ref(),
            native_instance_id,
            &expected_digest,
            &bridge_path,
            loaded,
        )
    }

    fn from_loaded(
        process: ExoProcessConfig,
        package_path: &Path,
        native_instance_id: &str,
        expected_digest: &str,
        bridge_path: &Path,
        loaded: Loaded,
    ) -> Result<Self, SyntheticInspectionError> {
        if loaded.digest != expected_digest || loaded.config.schema != "sts2.exo-one-shot-config-v2"
        {
            return Err(SyntheticInspectionError::ConfigurationMismatch);
        }
        let private_state = match &loaded.private_state {
            PrivateStateProfile::GuardedV2(policy) => policy.clone(),
            PrivateStateProfile::LegacyV1 => {
                return Err(SyntheticInspectionError::GuardedPrivateStateRequired);
            }
        };
        let artifact_paths = [
            loaded.config.executor.as_path(),
            loaded.config.extension.as_path(),
            loaded.config.node.as_path(),
        ];
        if !package_path.is_absolute() || artifact_paths.iter().any(|path| !path.is_absolute()) {
            return Err(SyntheticInspectionError::FileUnavailable);
        }
        let package = package_path
            .canonicalize()
            .map_err(|_| SyntheticInspectionError::FileUnavailable)?;
        let executor = loaded
            .config
            .executor
            .canonicalize()
            .map_err(|_| SyntheticInspectionError::FileUnavailable)?;
        if package != executor {
            return Err(SyntheticInspectionError::PackageMismatch);
        }
        let identity = loaded
            .inspected_identity(bridge_path, native_instance_id)
            .map_err(|_| SyntheticInspectionError::IdentityUnavailable)?;
        identity
            .validate_synthetic_loopback()
            .map_err(|_| SyntheticInspectionError::InvalidIdentity)?;
        if !identity.is_complete()
            || identity.source_revision != EXO_SOURCE_REVISION
            || identity.provider.as_deref() != Some("openai")
        {
            return Err(SyntheticInspectionError::IncompleteIdentity);
        }
        Ok(Self {
            process,
            identity,
            private_state,
        })
    }

    pub(crate) fn into_parts(self) -> (ExoProcessConfig, ExoIdentity, ExoPrivateStatePolicy) {
        (self.process, self.identity, self.private_state)
    }

    #[cfg(test)]
    pub(crate) fn structural_fixture(
        process: ExoProcessConfig,
        identity: ExoIdentity,
        private_state: ExoPrivateStatePolicy,
    ) -> Self {
        Self {
            process,
            identity,
            private_state,
        }
    }
}

fn launch_arguments(process: &ExoProcessConfig) -> Result<(&Path, &str), SyntheticInspectionError> {
    let [mode, configuration, digest] = process.arguments() else {
        return Err(SyntheticInspectionError::InvalidProcessConfiguration);
    };
    let configuration_path = Path::new(configuration);
    if mode != "--synthetic"
        || !configuration_path.is_absolute()
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SyntheticInspectionError::InvalidProcessConfiguration);
    }
    Ok((configuration_path, digest))
}

fn canonical_regular_file(path: &Path) -> Result<PathBuf, SyntheticInspectionError> {
    if !path.is_absolute() {
        return Err(SyntheticInspectionError::InvalidProcessConfiguration);
    }
    let metadata =
        std::fs::metadata(path).map_err(|_| SyntheticInspectionError::FileUnavailable)?;
    if !metadata.is_file() {
        return Err(SyntheticInspectionError::FileUnavailable);
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| SyntheticInspectionError::FileUnavailable)?;
    if canonical.as_path() != path {
        return Err(SyntheticInspectionError::FileUnavailable);
    }
    Ok(canonical)
}

/// Typed refusal reasons deliberately omit paths, config contents and provider material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyntheticInspectionError {
    InvalidProcessConfiguration,
    FileUnavailable,
    ConfigurationUnavailable,
    ConfigurationMismatch,
    GuardedPrivateStateRequired,
    PackageMismatch,
    IdentityUnavailable,
    InvalidIdentity,
    IncompleteIdentity,
}

impl std::fmt::Display for SyntheticInspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidProcessConfiguration => "synthetic Exo process configuration is invalid",
            Self::FileUnavailable => "synthetic Exo inspection file is unavailable",
            Self::ConfigurationUnavailable => "synthetic Exo config could not be inspected",
            Self::ConfigurationMismatch => "synthetic Exo config digest or schema does not match",
            Self::GuardedPrivateStateRequired => "synthetic Exo requires guarded private state",
            Self::PackageMismatch => "synthetic Exo package locator does not match config",
            Self::IdentityUnavailable => "synthetic Exo identity could not be inspected",
            Self::InvalidIdentity => "synthetic Exo identity is invalid",
            Self::IncompleteIdentity => "synthetic Exo identity is incomplete or unreviewed",
        })
    }
}

impl std::error::Error for SyntheticInspectionError {}

#[cfg(test)]
#[path = "synthetic_inspection_tests.rs"]
mod tests;
