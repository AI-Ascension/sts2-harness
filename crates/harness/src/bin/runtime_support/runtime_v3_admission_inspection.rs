// SPDX-License-Identifier: MIT

fn inspected_deployment(
    process: &ExoProcessConfig,
    package_path: &str,
    native_instance_id: &str,
) -> Result<ExoIdentity, String> {
    let [mode, path, digest] = process.arguments() else {
        return Err(
            "Exo envelope requires --run or --run-v2, absolute configuration path and digest"
                .to_owned(),
        );
    };
    if !matches!(mode.as_str(), "--run" | "--run-v2")
        || !std::path::Path::new(path).is_absolute()
    {
        return Err(
            "Exo envelope requires --run or --run-v2, absolute configuration path and digest"
                .to_owned(),
        );
    }
    let loaded = sts2_harness::exo_bridge_configuration::load(path)
        .map_err(|code| format!("Exo deployment inspection refused: {code}"))?;
    let package = std::fs::canonicalize(package_path)
        .map_err(|_| "Exo package locator is unavailable".to_owned())?;
    let executor = std::fs::canonicalize(&loaded.config.executor)
        .map_err(|_| "Exo configured executor is unavailable".to_owned())?;
    if &loaded.digest != digest || package != executor {
        return Err(
            "Exo deployment configuration or package locator does not match launch".to_owned(),
        );
    }
    loaded
        .inspected_identity(
            std::path::Path::new(process.executable()),
            native_instance_id,
        )
        .map_err(|code| format!("Exo deployment inspection refused: {code}"))
}

/// Inspects the artifacts the launch can bind to an explicit locator. The bridge executable and the
/// operator-located package artifact (`STS2_EXO_PACKAGE_PATH`) are each read whole through the
/// bounded `ExoInspectedArtifacts::read` and hashed into the inspected identity, so that identity
/// reflects the bytes actually on disk rather than the operator's declaration. A locator that is
/// missing, empty or unreadable is an error rather than an admission, so the seam stays fail-closed.
/// Every axis without inspected bytes stays unbound, and the reviewed preflight still refuses it as
/// `UnboundIdentity(axis)`.
fn inspected_artifacts(
    bridge_executable: &str,
    package_path: &str,
) -> Result<ExoInspectedArtifacts, String> {
    let bridge = ExoInspectedArtifacts::read(bridge_executable).map_err(|error| {
        format!("Exo admission cannot inspect the bridge artifact {bridge_executable}: {error}")
    })?;
    let package = ExoInspectedArtifacts::read(package_path).map_err(|error| {
        format!("Exo admission cannot inspect the package artifact {package_path}: {error}")
    })?;
    Ok(ExoInspectedArtifacts {
        package: Some(package),
        bridge: Some(bridge),
        ..ExoInspectedArtifacts::default()
    })
}

/// Admits the runtime bridge for the assembled deployment. The runtime takes its transport only
/// here, so it cannot dispatch one that the reviewed preflight has not admitted.
pub(super) fn admit(
    admission: &ExoRuntimeAdmission,
    process: ExoProcessConfig,
) -> Result<AdmittedExoRuntimeTransport<ExoProcessTransport>, String> {
    admission
        .admit(ExoProcessTransport::new(process))
        .map_err(String::from)
}

fn private_state_root() -> Result<String, String> {
    Ok(optional("STS2_EXO_PRIVATE_STATE_ROOT")?
        .unwrap_or_else(|| DEFAULT_PRIVATE_STATE_ROOT.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{ExoAdmissionMode, ExoInspectedArtifacts, inspected_artifacts, selected_mode};

    fn scratch_directory(name: &str) -> Result<PathBuf, String> {
        let path =
            std::env::temp_dir().join(format!("sts2-l139-package-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(path)
    }

    fn path_text(path: &Path) -> Result<&str, String> {
        path.to_str()
            .ok_or_else(|| format!("fixture path {path:?} is not valid UTF-8"))
    }

    fn write_bridge(root: &Path) -> Result<PathBuf, String> {
        let bridge = root.join("bridge-probe");
        std::fs::write(&bridge, b"bridge bytes").map_err(|error| error.to_string())?;
        Ok(bridge)
    }

    /// The package axis is the hash of the bytes at the locator, never of the operator's declared
    /// pin, so swapping the located artifact changes the inspected identity while the pin stays put.
    #[test]
    fn the_inspected_package_digest_is_the_hash_of_the_located_bytes() -> Result<(), String> {
        let root = scratch_directory("identity")?;
        let bridge = write_bridge(&root)?;
        let package = root.join("package-artifact");
        std::fs::write(&package, b"package bytes").map_err(|error| error.to_string())?;

        let inspected = inspected_artifacts(path_text(&bridge)?, path_text(&package)?)?;
        assert_eq!(
            inspected.package.as_deref().map(sts2_harness::sha256_hex),
            Some(sts2_harness::sha256_hex(b"package bytes"))
        );
        assert_eq!(
            inspected.bridge.as_deref().map(sts2_harness::sha256_hex),
            Some(sts2_harness::sha256_hex(b"bridge bytes"))
        );

        std::fs::write(&package, b"swapped package bytes").map_err(|error| error.to_string())?;
        let swapped = inspected_artifacts(path_text(&bridge)?, path_text(&package)?)?;
        assert_ne!(
            swapped.identity().package_digest,
            inspected.identity().package_digest
        );
        assert_eq!(
            swapped.identity().bridge_digest,
            inspected.identity().bridge_digest
        );
        std::fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(())
    }

    /// The inspection read is bounded, so an oversized package artifact is a fail-closed error rather
    /// than an unbounded allocation. The fixture is sparse, so it costs no disk space.
    #[test]
    fn an_oversized_package_artifact_is_a_bounded_error() -> Result<(), String> {
        let root = scratch_directory("bound")?;
        let bridge = write_bridge(&root)?;
        let package = root.join("oversized-package-artifact");
        std::fs::File::create(&package)
            .and_then(|file| file.set_len(ExoInspectedArtifacts::MAX_INSPECTED_ARTIFACT_BYTES + 1))
            .map_err(|error| error.to_string())?;

        let error = inspected_artifacts(path_text(&bridge)?, path_text(&package)?)
            .err()
            .ok_or("an oversized package artifact must refuse")?;
        assert!(
            error.contains("package artifact"),
            "the bound must name the package artifact, got {error}"
        );
        std::fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(())
    }

    /// A locator that names nothing on disk is refused rather than admitted with an unbound package
    /// axis.
    #[test]
    fn a_package_locator_that_names_no_file_is_refused() -> Result<(), String> {
        let root = scratch_directory("absent")?;
        let bridge = write_bridge(&root)?;

        let error = inspected_artifacts(path_text(&bridge)?, path_text(&root.join("absent"))?)
            .err()
            .ok_or("an absent package artifact must refuse")?;
        assert!(
            error.contains("package artifact"),
            "the refusal must name the package artifact, got {error}"
        );
        std::fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn admission_mode_defaults_to_the_reviewed_envelope_and_rejects_unknown_values() {
        assert_eq!(selected_mode(None), Ok(ExoAdmissionMode::Enveloped));
        assert_eq!(
            selected_mode(Some("envelope")),
            Ok(ExoAdmissionMode::Enveloped)
        );
        assert_eq!(selected_mode(Some("legacy")), Ok(ExoAdmissionMode::Legacy));
        for value in ["", "Envelope", "ENVELOPE", "legacy ", "admitted", "raw"] {
            assert!(
                selected_mode(Some(value)).is_err(),
                "{value:?} must be rejected"
            );
        }
    }
}
