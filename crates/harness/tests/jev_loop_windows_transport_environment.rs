// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! sts2-game-mod#79 acceptance guard for the environment the Windows lane's provider transport runs in.
//!
//! The runtime spawns the Exo bridge with the environment cleared but for `STS2_EXO_INHERITED_ENV_JSON`,
//! and the bridge hands its own environment to the transport it spawns, so that list is the transport's
//! whole environment. Here the transport is a `.cmd` that runs Windows PowerShell, and the list named
//! only the credential and the recording path.
//!
//! The failure that produced was invisible. Without `PATH` the command interpreter cannot find
//! `powershell.exe` at all, and the transport exits 9009 without making a call; with `PATH` but without
//! `SystemRoot` it finds the interpreter and the interpreter cannot load its own managed assemblies
//! (`0x8009001d`). Either way the transport exits non-zero before it reads its request, so it writes no
//! record, and the bridge and the runtime both spawn with the child's stderr discarded -- so the episode
//! ended as `episode policy decision was rejected: provider is unavailable` with no diagnostic anywhere,
//! while a direct request to the same endpoint from the same guest authenticated and answered.
//!
//! This test scans the real script, so dropping a variable the transport needs fails here instead of on
//! the next native launch. The negative cases below prove the guard is not vacuous.

use std::fs;
use std::path::PathBuf;

/// The transport's whole environment, and why each name is on the list.
const REQUIRED: [(&str, &str); 4] = [
    (
        "TYPESAFE_API_KEY",
        "without it the transport exits before the exchange",
    ),
    (
        "JEV_CONTEXT_LOG",
        "without it a completed exchange is not recorded",
    ),
    (
        "PATH",
        "without it the .cmd cannot resolve powershell.exe and exits 9009",
    ),
    (
        "SystemRoot",
        "without it Windows PowerShell cannot load its managed assemblies",
    ),
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

/// Every allowlist the lane declares, in file order.
///
/// The statement may be wrapped across lines, so a value is read from the first single-quoted run at
/// or after its assignment's first line. `PATH` is set once by the operating system before the lane
/// starts and the other three by the lane.
fn declared_environments(source: &str) -> Vec<String> {
    let lines = statements(source);
    let mut values = Vec::new();
    for index in 0..lines.len() {
        if lines[index].starts_with(DECLARATION)
            && let Some(value) = lines[index..].iter().find_map(|line| {
                let open = line.find('\'')?;
                let rest = &line[open + 1..];
                let close = rest.find('\'')?;
                Some(rest[..close].to_owned())
            })
        {
            values.push(value);
        }
    }
    values
}

/// The names in a declared allowlist, or the reason it is not a JSON array of strings.
fn declared_names(declared: &str) -> Result<Vec<String>, String> {
    let Some(inner) = declared
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return Err(format!(
            "STS2_EXO_INHERITED_ENV_JSON must be a JSON array of strings, found '{declared}'"
        ));
    };
    if inner.trim().is_empty() {
        return Err(
            "STS2_EXO_INHERITED_ENV_JSON is empty, so the transport gets nothing".to_owned(),
        );
    }
    Ok(inner
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .collect())
}

/// Every way the Windows lane can stop giving its provider transport what it needs.
fn environment_violations(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let declared = declared_environments(source);
    if declared.is_empty() {
        found.push(
            "the Windows lane no longer declares STS2_EXO_INHERITED_ENV_JSON, so the transport \
             receives a cleared environment"
                .to_owned(),
        );
        return found;
    }
    // A second declaration is not a second list: the process environment holds one value, so a lane
    // that appends to the statement widens the clearance without changing the first one.
    if declared.len() > 1 {
        found.push(format!(
            "the transport's environment is declared {} times; the process environment holds one \
             value, so an appended declaration silently replaces this list",
            declared.len()
        ));
    }
    for declared in &declared {
        found.extend(declared_environment_violations(declared));
    }
    found
}

/// The ways one declared allowlist fails to be the transport's whole environment.
fn declared_environment_violations(declared: &str) -> Vec<String> {
    let mut found = Vec::new();
    let names = match declared_names(declared) {
        Ok(names) => names,
        Err(reason) => return vec![reason],
    };

    for (name, reason) in REQUIRED {
        if !names.iter().any(|declared| declared == name) {
            found.push(format!("the transport is no longer given {name}: {reason}"));
        }
    }

    // The list is a clearance, not a convenience: it is the transport's whole environment, and every
    // name on it is a name the operator has decided the exchange needs.
    for name in &names {
        if !REQUIRED.iter().any(|(required, _)| required == name) {
            found.push(format!(
                "{name} is not a name the transport needs, and the list's whole purpose is that the \
                 transport receives nothing else"
            ));
        }
    }
    if names.len() != REQUIRED.len() {
        found.push(format!(
            "the transport's environment must name each required variable exactly once, found {} \
             entries",
            names.len()
        ));
    }

    found
}

fn replace(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture line not found: {from}");
    source.replace(from, to)
}

const DECLARATION: &str = "$env:STS2_EXO_INHERITED_ENV_JSON =";

#[test]
fn windows_lane_gives_its_provider_transport_the_environment_the_transport_needs() {
    let source = launcher();
    let found = environment_violations(&source);
    assert!(found.is_empty(), "{}", found.join("; "));
}

#[test]
fn transport_environment_guard_rejects_a_missing_or_widened_allowlist() {
    let source = launcher();

    // Dropping any name the transport needs, one at a time.
    for (name, _) in REQUIRED {
        let narrowed = replace(&source, &format!("\"{name}\""), &format!("\"NOT_{name}\""));
        assert!(
            !environment_violations(&narrowed).is_empty(),
            "dropping {name} must be caught"
        );
    }

    // Deleting the declaration outright.
    let deleted = source
        .lines()
        .filter(|line| !line.trim().starts_with(DECLARATION))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !environment_violations(&deleted).is_empty(),
        "a lane that declares no environment at all must be caught"
    );

    // A list that is not an array: the runtime rejects the value outright, so the lane never starts.
    let not_an_array = replace(
        &source,
        "'[\"TYPESAFE_API_KEY\",\"JEV_CONTEXT_LOG\",\"PATH\",\"SystemRoot\"]'",
        "'TYPESAFE_API_KEY'",
    );
    assert!(
        !environment_violations(&not_an_array).is_empty(),
        "the runtime requires a JSON array of strings"
    );

    // An exchange does not need the operator's profile, and granting it would undo the clearance.
    let widened = replace(
        &source,
        "\"SystemRoot\"]'",
        "\"SystemRoot\",\"USERPROFILE\"]'",
    );
    assert!(
        !environment_violations(&widened).is_empty(),
        "the allowlist must stay the transport's whole environment and nothing more"
    );

    // Appending a second declaration, which is how a list grows without the first one changing.
    let duplicated = replace(
        &source,
        DECLARATION,
        &format!(
            "{DECLARATION}\n        $env:STS2_EXO_INHERITED_ENV_JSON = \
             '[\"TYPESAFE_API_KEY\"]'"
        ),
    );
    assert!(
        !environment_violations(&duplicated).is_empty(),
        "a lane that declares the environment twice must be caught"
    );
}
