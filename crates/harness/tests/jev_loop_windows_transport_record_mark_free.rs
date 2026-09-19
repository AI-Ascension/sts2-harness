// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! sts2-game-mod#173 acceptance guard for the exchange record the Windows transport writes.
//!
//! `JEV_CONTEXT_LOG` names a JSON Lines file: one exchange per line, read line by line. PowerShell's
//! `[Text.Encoding]::UTF8` is a `UTF8Encoding` with the identifier turned on, so it writes a
//! byte-order mark when it *creates* the file -- `AppendAllText` only emits the preamble on the
//! create, which is why the mark appears once, in front of the first record, rather than on every
//! append. A reader that opens the file as UTF-8 and parses line one as JSON fails there.
//!
//! Observed on the guest: `runs\episode-20260919-100810\jev-context.jsonl` is 324,031 B and its
//! first three bytes are `EF BB BF` (the only mark in the file), and the first line is a complete
//! exchange record -- so the mark, not the record, is what a strict JSON Lines reader rejects.
//!
//! This test scans the real script. The negative cases below prove the guard is not vacuous.

use std::fs;
use std::path::PathBuf;

/// The append that grows the record, and the encoding object it must be handed.
const RECORD_APPEND: &str = "[IO.File]::AppendAllText($path, $line + \"`n\", $script:MarkFreeUtf8)";

/// Spellings that carry the mark, either because the encoding object emits it or because the cmdlet
/// does so on its own under this interpreter when told `-Encoding UTF8`.
const MARKED: [&str; 5] = [
    "[Text.Encoding]::UTF8)",
    "-Encoding UTF8",
    "-Encoding utf8",
    "UTF8Encoding($true)",
    "UTF8Encoding()",
];

fn transport_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/jev-plays-sts2/systemone_transport.ps1")
}

fn transport() -> String {
    let path = transport_path();
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Trims a line so the assertions compare statements rather than indentation.
fn statements(source: &str) -> Vec<String> {
    source.lines().map(|line| line.trim().to_owned()).collect()
}

/// Every way the transport can start marking the record stream again.
fn mark_violations(source: &str) -> Vec<String> {
    let lines = statements(source);
    let mut found = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        // A comment may name the mark or the cmdlet that emits it; only a statement can write one.
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if MARKED.iter().any(|marked| line.contains(marked)) {
            found.push(format!(
                "line {} writes the record with an encoding that emits a byte-order mark: {line}",
                index + 1
            ));
        }
    }

    if !lines.iter().any(|line| line.starts_with(RECORD_APPEND)) {
        found.push(format!(
            "'{RECORD_APPEND}' is gone, so the record is no longer appended mark-free"
        ));
    }
    if !source.contains("New-Object System.Text.UTF8Encoding($false)") {
        found.push(
            "the record writer must construct UTF8Encoding with $false; any other constructor \
             emits the byte-order mark in front of the first record"
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
fn windows_transport_records_every_exchange_without_a_byte_order_mark() {
    let source = transport();
    let found = mark_violations(&source);
    assert!(found.is_empty(), "{}", found.join("; "));
}

#[test]
fn transport_record_guard_rejects_a_marked_or_rewired_write() {
    let source = transport();

    // The write as it was, which is what put the mark in front of the first record.
    let marked = replace(
        &source,
        RECORD_APPEND,
        "[IO.File]::AppendAllText($path, $line + \"`n\", [Text.Encoding]::UTF8)",
    );
    assert!(
        !mark_violations(&marked).is_empty(),
        "the encoding this guard exists for must be caught"
    );

    // The same encoding asked for by name one constructor argument away from mark-free.
    let emitting = replace(
        &source,
        "New-Object System.Text.UTF8Encoding($false)",
        "New-Object System.Text.UTF8Encoding($true)",
    );
    assert!(
        !mark_violations(&emitting).is_empty(),
        "a UTF8Encoding that emits the identifier is the defect, not a variation of the fix"
    );

    // A cmdlet spelling that writes the mark without any encoding object at all.
    let out_file = replace(
        &source,
        RECORD_APPEND,
        "$line | Out-File -FilePath $path -Append -Encoding UTF8",
    );
    assert!(
        !mark_violations(&out_file).is_empty(),
        "the guard must see the cmdlets, not only the encoding object"
    );

    // Deleting the record write: an exchange that records nothing cannot be diagnosed at all.
    let deleted = source
        .lines()
        .filter(|line| !line.trim().starts_with(RECORD_APPEND))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !mark_violations(&deleted).is_empty(),
        "a transport that stops recording must be caught"
    );
}
