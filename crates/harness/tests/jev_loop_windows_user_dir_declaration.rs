// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! sts2-game-mod#173 acceptance guard for the Windows Jev lane.
//!
//! The production live runtime refuses to initialise unless the directory the game resolved equals
//! the directory the launcher published in `STS2_LIVE_USER_DIR`. The Linux lane starts the game
//! through the supported launcher, which sets that variable from `--user-dir-mapping`. The Windows
//! lane launches the host executable directly, so the variable is the lane's own responsibility --
//! and it was never set, which made every episode that reached the mod fail with "live demo requires
//! its isolated user directory" while the game was resolving exactly the directory the lane had
//! isolated.
//!
//! This test is the executable form of that wiring: the declaration must exist, must be the value
//! read back from the `override.cfg` the lane just wrote rather than the name it wrote into it, must
//! be the same value that gets seeded, and must be cleared with the rest of the episode environment.
//! It scans the real script, so removing the declaration fails here instead of on the next native
//! launch, and the negative cases below prove the guard is not vacuous.

use std::fs;
use std::path::PathBuf;

fn launcher_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../experiments/jev-plays-sts2/jev-loop.ps1")
}

fn launcher() -> String {
    let path = launcher_path();
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Trims a line so the assertions compare statements rather than indentation.
fn statements(source: &str) -> Vec<String> {
    source.lines().map(|line| line.trim().to_owned()).collect()
}

/// Every way the Windows lane can stop declaring the isolated user directory correctly.
fn declaration_violations(source: &str) -> Vec<String> {
    let lines = statements(source);
    let mut found = Vec::new();

    let declared: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("$env:STS2_LIVE_USER_DIR"))
        .collect();
    if declared.is_empty() {
        found.push("the Windows lane never sets STS2_LIVE_USER_DIR".to_owned());
    }
    for line in &declared {
        if *line != "$env:STS2_LIVE_USER_DIR = $userDirPath" {
            found.push(format!(
                "STS2_LIVE_USER_DIR must be declared from the resolved directory, found '{line}'"
            ));
        }
    }

    let resolved = "$userDirPath = Resolve-IsolatedUserDirectory -HostDirectory $hostDir";
    if !lines.iter().any(|line| line == resolved) {
        found.push(format!(
            "the declared directory must be read back from override.cfg; expected '{resolved}'"
        ));
    }

    let seeded = "Initialize-IsolatedUserDir -Target $userDirPath";
    if !lines.iter().any(|line| line == seeded) {
        found.push(format!(
            "the seeded directory and the declared directory must be one value; expected '{seeded}'"
        ));
    }

    if !source.contains("function Resolve-IsolatedUserDirectory {") {
        found.push("the resolver that reads the override back is missing".to_owned());
    }
    if !source.contains("config/custom_user_dir_name") {
        found.push("the resolver does not read config/custom_user_dir_name".to_owned());
    }
    if !source.contains("Join-Path $env:APPDATA $name") {
        found.push(
            "the resolver must resolve the override name under the launch-scoped APPDATA root"
                .to_owned(),
        );
    }

    let cleared: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("'STS2_LIVE_USER_DIR'"))
        .collect();
    if cleared.is_empty() {
        found.push("the episode environment cleanup does not clear STS2_LIVE_USER_DIR".to_owned());
    }

    found
}

fn replace(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture line not found: {from}");
    source.replace(from, to)
}

#[test]
fn windows_lane_declares_the_isolated_user_directory() {
    let source = launcher();
    let found = declaration_violations(&source);
    assert!(found.is_empty(), "{}", found.join("; "));
}

#[test]
fn declaration_guard_rejects_a_removed_or_rewired_declaration() {
    let source = launcher();

    let removed = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("$env:STS2_LIVE_USER_DIR = "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!declaration_violations(&removed).is_empty(), "removal");

    let unresolved = replace(
        &source,
        "$env:STS2_LIVE_USER_DIR = $userDirPath",
        "$env:STS2_LIVE_USER_DIR = $userDir",
    );
    assert!(
        !declaration_violations(&unresolved).is_empty(),
        "the name written into override.cfg is not the resolved directory"
    );

    let no_read_back = replace(
        &source,
        "$userDirPath = Resolve-IsolatedUserDirectory -HostDirectory $hostDir",
        "$userDirPath = $userDir",
    );
    assert!(
        !declaration_violations(&no_read_back).is_empty(),
        "the declaration must not be assumed from the name that was written"
    );

    let drifted_seed = replace(
        &source,
        "Initialize-IsolatedUserDir -Target $userDirPath",
        "Initialize-IsolatedUserDir -Target $userDir",
    );
    assert!(
        !declaration_violations(&drifted_seed).is_empty(),
        "seeded and declared directories must be one value"
    );

    let uncleared = replace(&source, "'STS2_LIVE_USER_DIR', ", "");
    assert!(
        !declaration_violations(&uncleared).is_empty(),
        "the episode cleanup must clear the declaration"
    );

    let unnamed = source.replace("config/custom_user_dir_name", "custom_user_dir_name");
    assert!(
        !declaration_violations(&unnamed).is_empty(),
        "the resolver must read the override key the game reads"
    );
}
