// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::{ExoInspectedArtifacts, inspected_artifacts};

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
