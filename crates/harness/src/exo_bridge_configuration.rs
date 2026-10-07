// SPDX-License-Identifier: MIT

use crate::{
    EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoIdentity, ExoPrivateStatePolicy, ExoToolCatalog,
    responses_capable, sha256_hex,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[path = "exo_bridge_configuration/capability.rs"]
mod capability;

#[path = "exo_bridge_configuration/digest.rs"]
mod digest;

#[path = "exo_bridge_configuration/synthetic_inspection.rs"]
mod synthetic_inspection;

use digest::bridge_digest;

pub use capability::{
    LOOKUP_SUPPORTED_DECISIONS, LOOKUP_UNSUPPORTED_DECISIONS, PROVIDER_ENDPOINT,
    SUPPORTED_CONTEXT_MODES, SUPPORTED_DECISIONS, SUPPORTED_PROFILES, SYNTHETIC_ENDPOINT_PREFIX,
    SYNTHETIC_MODEL, UNSUPPORTED_DECISIONS, UNSUPPORTED_PROFILE_CODE, UNSUPPORTED_PROFILES,
    UNSUPPORTED_RECOVERY_CODE, UnsupportedProfileAxis, capability_fields, provider_route_admitted,
    synthetic_route_admitted, unsupported_profile_axis,
};

pub use synthetic_inspection::{SyntheticInspectionError, SyntheticLoopbackInspection};

pub const MAX_EXECUTOR_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_EXTENSION_BYTES: usize = 64 * 1024;
pub const MAX_NODE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_CONFIGURATION_BYTES: usize = 32 * 1024;
/// The bound for hashing the *running* executable when advertising a profile.
///
/// #785: this is deliberately not [`MAX_EXECUTOR_BYTES`]. That constant is a production boundary —
/// it is how large a shipped bridge package may be before it is refused — and reusing it here made
/// an unrelated build-size increase fail advertisement tests with `exo_bridge_package_bound`, a
/// packaging error reported by tests that only assert wire-format invariants. The harness debug
/// test binary is already ~182 MiB, so that coupling was 36% consumed by a single crate.
///
/// Raising this is not a way to remove the check. It exists so that a test binary which outgrows it
/// is refused as `exo_bridge_advertised_executable_bound`, which names the actual subject, instead
/// of as a bridge package that was never packaged.
pub const MAX_ADVERTISED_EXECUTABLE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema: String,
    pub executor: PathBuf,
    pub executor_sha256: String,
    pub source_root: PathBuf,
    pub extension: PathBuf,
    pub extension_sha256: String,
    pub node: PathBuf,
    pub node_sha256: String,
    pub model: String,
    pub endpoint: String,
}

pub struct Loaded {
    pub config: Configuration,
    pub digest: String,
    pub private_state: PrivateStateProfile,
}

/// Closed configuration selection. Legacy v1 retains its historical lexical-only behavior;
/// guarded v2 binds an exact policy and cannot fall back to v1 when materialization fails.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrivateStateProfile {
    LegacyV1,
    GuardedV2(ExoPrivateStatePolicy),
}

impl Loaded {
    #[must_use]
    pub const fn guarded_private_state(&self) -> Option<&ExoPrivateStatePolicy> {
        match &self.private_state {
            PrivateStateProfile::LegacyV1 => None,
            PrivateStateProfile::GuardedV2(policy) => Some(policy),
        }
    }
}

#[derive(Deserialize)]
struct SchemaHeader {
    schema: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardedConfiguration {
    schema: String,
    executor: PathBuf,
    executor_sha256: String,
    source_root: PathBuf,
    extension: PathBuf,
    extension_sha256: String,
    node: PathBuf,
    node_sha256: String,
    model: String,
    endpoint: String,
    private_state: ExoPrivateStatePolicy,
}

pub fn load(path: &str) -> Result<Loaded, &'static str> {
    load_profile(path, false)
}

#[path = "exo_bridge_configuration/profile.rs"]
mod profile;

pub fn load_profile(path: &str, lookup: bool) -> Result<Loaded, &'static str> {
    profile::load_profile(path, lookup)
}
