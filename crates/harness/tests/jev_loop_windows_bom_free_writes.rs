// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! sts2-game-mod#173 acceptance guard for the files the Windows Jev lane writes.
//!
//! The lane runs under Windows PowerShell 5.1, and there `Set-Content ... -Encoding UTF8` means
//! UTF-8 *with* a byte-order mark. The game does not accept that mark in `override.cfg`: it does not
//! honour `config/use_custom_user_dir`, resolves the shared default user directory instead of the
//! one the file names, and then the mod compares the directory the lane declared with the directory
//! the game resolved, finds they disagree, and refuses to initialise with
//! `live demo requires its isolated user directory`.
//!
//! That is not a theory about the parser. Of the 212 episodes the guest still holds, the lane drove
//! 18 and every one of them resolved `AppData\Roaming\SlayTheSpire2`; the 167 driven by the manual
//! probes, which write the same four lines without a mark, all resolved their own
//! `AIAscensionJevLoop-<stamp>` directory. The separation is total, in both directions, and the only
//! difference between the two writers is the mark.
//!
//! The lane therefore writes every file another program reads through `Write-TextFile`, which
//! constructs `UTF8Encoding($false)`. This test scans the real script, so reintroducing the mark one
//! call at a time fails here instead of on the next native launch, and the negative cases below prove
//! the guard is not vacuous.

use std::fs;
use std::path::PathBuf;

/// Files the lane writes for another program to read, and the call that must write each one.
const THROUGH_THE_HELPER: [&str; 3] = [
    "Write-TextFile (Join-Path $hostDir 'override.cfg')",
    "Write-TextFile $settingsPath",
    "Write-TextFile (Join-Path $runDir 'authorization.json')",
];

/// Cmdlets that emit a byte-order mark under this interpreter when told `-Encoding UTF8`.
const MARKED_WRITERS: [&str; 3] = ["Set-Content", "Add-Content", "Out-File"];

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

/// Every way the Windows lane can start handing a marked file to the game again.
fn bom_violations(source: &str) -> Vec<String> {
    let lines = statements(source);
    let mut found = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if !line.to_lowercase().contains("-encoding utf8") {
            continue;
        }
        if MARKED_WRITERS.iter().any(|cmdlet| line.contains(cmdlet)) {
            found.push(format!(
                "line {} writes a byte-order mark the game does not accept: {line}",
                index + 1
            ));
        }
    }

    for expected in THROUGH_THE_HELPER {
        if !lines.iter().any(|line| line.starts_with(expected)) {
            found.push(format!(
                "'{expected}' is gone, so that file is no longer written mark-free"
            ));
        }
    }

    if !source.contains("[IO.File]::WriteAllText(") {
        found.push("the mark-free writer no longer writes through WriteAllText".to_owned());
    }
    if !source.contains("UTF8Encoding($false)") {
        found.push(
            "the mark-free writer must construct UTF8Encoding with $false; any other encoding \
             constructor emits the byte-order mark the game ignores"
                .to_owned(),
        );
    }

    found
}

fn replace(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture line not found: {from}");
    source.replace(from, to)
}

#[test]
fn windows_lane_writes_every_shared_file_without_a_byte_order_mark() {
    let source = launcher();
    let found = bom_violations(&source);
    assert!(found.is_empty(), "{}", found.join("; "));
}

#[test]
fn byte_order_mark_guard_rejects_a_marked_or_rewired_write() {
    let source = launcher();

    // The write the two defects are about, put back the way it was: piped into Set-Content with the
    // encoding that emits the mark.
    let marked_override = replace(
        &source,
        "Write-TextFile (Join-Path $hostDir 'override.cfg') $overrideText",
        "$overrideText | Set-Content -LiteralPath (Join-Path $hostDir 'override.cfg') -Encoding UTF8",
    );
    assert!(
        !bom_violations(&marked_override).is_empty(),
        "a marked override.cfg is exactly the failure this guard exists for"
    );

    let marked_settings = replace(
        &source,
        "Write-TextFile $settingsPath ($settings | ConvertTo-Json -Depth 30)",
        "$settings | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $settingsPath -Encoding UTF8",
    );
    assert!(
        !bom_violations(&marked_settings).is_empty(),
        "the seeded settings.save is read by the same parser and must be mark-free too"
    );

    // Other spellings of the same mistake.
    let marked_constant = replace(
        &source,
        "Write-TextFile (Join-Path $runDir 'authorization.json') (@{",
        "@{",
    );
    assert!(
        !bom_violations(&marked_constant).is_empty(),
        "an authorization.json written some other way must be caught"
    );

    let marked_out_file = replace(
        &source,
        "Write-TextFile (Join-Path $runDir 'authorization.json') (@{",
        ("$record | Out-File -FilePath (Join-Path $runDir 'authorization.json') -Encoding UTF8"
            .to_owned()
            + "\n@{")
            .as_str(),
    );
    assert!(
        !bom_violations(&marked_out_file).is_empty(),
        "the guard must see every mark-emitting cmdlet, not only Set-Content"
    );

    // The helper itself: a mark-free writer has to ask for the mark-free constructor.
    let marked_helper = replace(&source, "UTF8Encoding($false)", "UTF8Encoding($true)");
    assert!(
        !bom_violations(&marked_helper).is_empty(),
        "the helper's own encoding choice is the invariant"
    );

    let helper_deleted = replace(
        &source,
        "[IO.File]::WriteAllText($Path, \"$Text`r`n\", (New-Object System.Text.UTF8Encoding($false)))",
        "Set-Content -LiteralPath $Path -Value $Text -Encoding UTF8",
    );
    assert!(
        !bom_violations(&helper_deleted).is_empty(),
        "a helper that writes with Set-Content cannot be mark-free"
    );
}
