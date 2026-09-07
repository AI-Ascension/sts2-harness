// SPDX-License-Identifier: MIT

use std::fs;
use std::path::PathBuf;

use super::{fingerprint_component, mcp_executable};

fn fixture_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sts2-harness-completed-resume-{name}-{}",
        std::process::id()
    ))
}

#[test]
fn resume_requires_every_fingerprint_component() -> Result<(), String> {
    assert!(
        fingerprint_component("STS2_SEED", Some("STS2_VISIBLE_SEED"), "fallback", true).is_err()
    );
    assert!(fingerprint_component("STS2_BUILD_DIGEST", None, "fallback", true).is_err());
    assert!(fingerprint_component("STS2_STATE_DIGEST", None, "fallback", true).is_err());
    let new_episode = fingerprint_component("STS2_BUILD_DIGEST", None, "fallback", false)?;
    assert_eq!(new_episode, super::digest_text("fallback"));
    Ok(())
}

#[test]
fn mcp_executable_digest_requires_a_bounded_regular_file() -> Result<(), String> {
    let path = fixture_path("regular");
    fs::write(&path, b"synthetic mcp executable")
        .map_err(|error| format!("cannot write fixture: {error}"))?;
    let value = mcp_executable(
        path.to_str()
            .ok_or_else(|| String::from("fixture path is not UTF-8"))?,
    )?;
    assert_eq!(value["bytes"], 24);
    assert!(value["sha256"].as_str().is_some());
    fs::remove_file(path).map_err(|error| format!("cannot remove fixture: {error}"))?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn mcp_executable_digest_rejects_a_symbolic_link() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let target = fixture_path("target");
    let link = fixture_path("link");
    fs::write(&target, b"synthetic mcp executable")
        .map_err(|error| format!("cannot write fixture: {error}"))?;
    symlink(&target, &link).map_err(|error| format!("cannot create symlink: {error}"))?;
    let result = mcp_executable(
        link.to_str()
            .ok_or_else(|| String::from("link path is not UTF-8"))?,
    );
    assert!(result.is_err());
    fs::remove_file(link).map_err(|error| format!("cannot remove link: {error}"))?;
    fs::remove_file(target).map_err(|error| format!("cannot remove target: {error}"))?;
    Ok(())
}
