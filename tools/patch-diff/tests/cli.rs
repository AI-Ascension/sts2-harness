// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const MANIFEST: &str = r#"{"quarantine":{"status":"quarantined"}}"#;
static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "sts2-patch-diff-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn run(&self, before: &[u8], after: &[u8]) -> std::io::Result<Output> {
        fs::write(self.0.join("before.json"), before)?;
        fs::write(self.0.join("after.json"), after)?;
        Command::new(env!("CARGO_BIN_EXE_sts2-patch-diff"))
            .arg(self.0.join("before.json"))
            .arg(self.0.join("after.json"))
            .output()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("before.json"));
        let _ = fs::remove_file(self.0.join("after.json"));
        let _ = fs::remove_dir(&self.0);
    }
}

#[test]
fn unchanged_cli_output_is_deterministic_and_line_reporting_is_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.run(MANIFEST.as_bytes(), MANIFEST.as_bytes())?;
    assert!(first.status.success());
    assert!(first.stderr.is_empty());
    assert_eq!(
        first.stdout,
        fixture
            .run(MANIFEST.as_bytes(), MANIFEST.as_bytes())?
            .stdout
    );
    let report: Value = serde_json::from_slice(&first.stdout)?;
    assert_eq!(report["changed_line_count"], 0);
    assert_eq!(report["truncated"], false);
    assert_eq!(
        report["before"]["fingerprint"],
        report["after"]["fingerprint"]
    );
    assert_eq!(report["after"]["quarantine_status"], "quarantined");

    let shifted = format!("{}{MANIFEST}", "\n".repeat(130));
    let output = fixture.run(MANIFEST.as_bytes(), shifted.as_bytes())?;
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["changed_line_count"], 131);
    assert_eq!(report["truncated"], true);
    let lines = report["reported_line_numbers"]
        .as_array()
        .ok_or("missing lines")?;
    assert_eq!(lines.len(), 128);
    assert_eq!(lines[0], 1);
    assert_eq!(lines[127], 128);
    Ok(())
}

#[test]
fn invalid_or_oversized_input_fails_without_emitting_a_report()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    for input in [b"invalid JSON".as_slice(), &vec![b' '; 512 * 1024 + 1]] {
        let result = fixture.run(MANIFEST.as_bytes(), input)?;
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(!result.stderr.is_empty());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_sts2-patch-diff")).output()?;
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    Ok(())
}
