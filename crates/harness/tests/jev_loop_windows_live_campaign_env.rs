// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! sts2-game-mod#79 acceptance guard for the Windows Jev lane.
//!
//! The mod's production live runtime is opt-in: `STS2_LIVE_COMBAT=1` is the switch that binds the
//! gameplay host, `STS2_LIVE_CAMPAIGN=1` selects the isolated campaign save backend, and
//! `STS2_LIVE_CAMPAIGN_MODE=standard` selects the standard seeded run path. The reviewed launcher
//! sets all three in the child environment and the Linux lane inherits them that way. The Windows
//! lane launches the host executable directly, so the environment is its own responsibility -- and
//! it declared only the runtime session variables.
//!
//! The failure that produced was not a refusal. The mod loaded and opened its listener, then served
//! every read as `{"state":"recovery","code":"host_not_configured"}` -- the code its host seam
//! reports when no host was ever configured -- and the harness ended the episode as `episode
//! requires recovery before policy can continue`. Nothing in that message names the environment, so
//! the listener evidence looked like progress.
//!
//! This test is the executable form of that wiring: each gate must be declared, must carry the value
//! the mod compares against, and must be declared before the game is started, because an environment
//! this process sets after the launch reaches nothing. It scans the real script, so removing a
//! declaration fails here instead of on the next native launch, and the negative cases below prove
//! the guard is not vacuous.

use std::fs;
use std::path::PathBuf;

/// The three declarations the live runtime needs, as the lane must write them.
const REQUIRED: [&str; 3] = [
    "$env:STS2_LIVE_COMBAT = '1'",
    "$env:STS2_LIVE_CAMPAIGN = '1'",
    "$env:STS2_LIVE_CAMPAIGN_MODE = 'standard'",
];

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

/// Every way the Windows lane can stop opting into the live runtime correctly.
fn env_violations(source: &str) -> Vec<String> {
    let lines = statements(source);
    let mut found = Vec::new();

    let launch = lines
        .iter()
        .position(|line| line.starts_with("$gameProc = Start-Process $game"));
    if launch.is_none() {
        found.push("the lane no longer starts the game with Start-Process".to_owned());
    }

    for expected in REQUIRED {
        let name = expected
            .split_whitespace()
            .next()
            .and_then(|token| token.strip_prefix("$env:"))
            .unwrap_or(expected);
        let prefix = format!("$env:{name} =");
        let declared: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with(&prefix))
            .map(|(index, _)| index)
            .collect();
        if declared.is_empty() {
            found.push(format!("the Windows lane never sets {name}"));
            continue;
        }
        for index in &declared {
            if lines[*index] != expected {
                found.push(format!(
                    "{name} must be declared as '{expected}', found '{}'",
                    lines[*index]
                ));
            }
        }
        if let Some(at) = launch
            && declared.iter().any(|index| *index > at)
        {
            found.push(format!(
                "{name} is declared after the game has been started, so the game cannot see it"
            ));
        }
    }

    // Standard campaign mode rejects a supplied seed, and the mod throws rather than ignoring one.
    if lines
        .iter()
        .any(|line| line.starts_with("$env:STS2_LIVE_SEED ="))
    {
        found.push(
            "standard campaign mode rejects a supplied seed; STS2_LIVE_SEED must not be set"
                .to_owned(),
        );
    }

    found
}

fn replace(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture line not found: {from}");
    source.replace(from, to)
}

fn without(source: &str, statement: &str) -> String {
    assert!(
        source.contains(statement),
        "fixture line not found: {statement}"
    );
    source
        .lines()
        .filter(|line| line.trim() != statement)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn windows_lane_opts_into_the_live_runtime() {
    let source = launcher();
    let found = env_violations(&source);
    assert!(found.is_empty(), "{}", found.join("; "));
}

#[test]
fn live_runtime_guard_rejects_a_missing_or_rewired_declaration() {
    let source = launcher();

    for statement in REQUIRED {
        let removed = without(&source, statement);
        assert!(
            !env_violations(&removed).is_empty(),
            "removing {statement} must be caught"
        );
    }

    let not_opted_in = replace(
        &source,
        "$env:STS2_LIVE_COMBAT = '1'",
        "$env:STS2_LIVE_COMBAT = '0'",
    );
    assert!(
        !env_violations(&not_opted_in).is_empty(),
        "the gate must carry the value the mod compares against"
    );

    let not_campaign = replace(
        &source,
        "$env:STS2_LIVE_CAMPAIGN = '1'",
        "$env:STS2_LIVE_CAMPAIGN = 'true'",
    );
    assert!(
        !env_violations(&not_campaign).is_empty(),
        "the campaign switch must carry the value the mod compares against"
    );

    let renamed_mode = replace(
        &source,
        "$env:STS2_LIVE_CAMPAIGN_MODE = 'standard'",
        "$env:STS2_CAMPAIGN_MODE = 'standard'",
    );
    assert!(
        !env_violations(&renamed_mode).is_empty(),
        "the standard mode must be declared under the name the mod reads"
    );

    let practice_mode = replace(
        &source,
        "$env:STS2_LIVE_CAMPAIGN_MODE = 'standard'",
        "$env:STS2_LIVE_CAMPAIGN_MODE = 'practice'",
    );
    assert!(
        !env_violations(&practice_mode).is_empty(),
        "practice mode requires an explicit seed and is not the standard run path"
    );

    let seed_supplied = replace(
        &source,
        "$env:STS2_LIVE_CAMPAIGN_MODE = 'standard'",
        "$env:STS2_LIVE_CAMPAIGN_MODE = 'standard'\n$env:STS2_LIVE_SEED = 'A79TEST20260916'",
    );
    assert!(
        !env_violations(&seed_supplied).is_empty(),
        "standard mode must not be handed a seed"
    );

    let started_too_early = format!(
        "{}\n$env:STS2_LIVE_COMBAT = '1'\n",
        without(&source, "$env:STS2_LIVE_COMBAT = '1'")
    );
    assert!(
        !env_violations(&started_too_early).is_empty(),
        "an environment set after the launch reaches nothing"
    );
}
