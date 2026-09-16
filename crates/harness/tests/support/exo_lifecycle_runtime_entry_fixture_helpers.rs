// SPDX-License-Identifier: MIT

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use sts2_harness::exo_lifecycle::EXO_LIFECYCLE_WIRE_V2;
use sts2_harness::provider_session::NativeCapabilities;
use sts2_harness::{ExoIdentity, sha256_hex};

pub fn lifecycle_capabilities(identity: &ExoIdentity) -> Result<NativeCapabilities, String> {
    let executor = required_axis(identity.package_digest.as_deref())?.to_owned();
    let configuration = required_axis(identity.config_digest.as_deref())?;
    let profile = sha256_hex(
        serde_json::to_vec(&serde_json::json!({
            "adapter":"sts2-exo-lifecycle-v2",
            "configuration_sha256":configuration,
            "executor_sha256":executor,
            "wire":EXO_LIFECYCLE_WIRE_V2
        }))
        .map_err(|error| error.to_string())?,
    );
    NativeCapabilities::reviewed_exo_lifecycle(
        "sts2-exo-lifecycle-v2",
        profile,
        executor,
        sha256_hex(EXO_LIFECYCLE_WIRE_V2),
    )
    .map_err(|error| error.to_string())
}

pub(super) fn required_axis(value: Option<&str>) -> Result<&str, String> {
    value.ok_or_else(|| String::from("inspected deployment identity omitted an axis"))
}

pub(super) fn write_executable(path: &Path, source: &str) -> Result<(), String> {
    std::fs::write(path, source).map_err(|error| error.to_string())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())
}
