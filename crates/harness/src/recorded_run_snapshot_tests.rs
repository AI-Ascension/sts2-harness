// SPDX-License-Identifier: MIT

use super::tests::{root, write_fixture};
use super::*;
use std::fs;

#[test]
fn symlink_child_root_and_replaced_snapshot_fail() -> Result<(), String> {
    let input = root("snapshot-review");
    write_fixture(&input, false)?;
    // Unrelated database/log storage is not an input and may be large or unavailable.
    let database = fs::File::create(input.join("recording.sqlite")).map_err(|e| e.to_string())?;
    database
        .set_len(128 * 1024 * 1024)
        .map_err(|e| e.to_string())?;
    fs::create_dir(input.join("logs")).map_err(|e| e.to_string())?;
    std::os::unix::fs::symlink("/missing-unrelated", input.join("ignored.log"))
        .map_err(|e| e.to_string())?;
    let snapshot = Snapshot::read(&input)?;
    assert_eq!(snapshot.files.len(), 6);
    snapshot.verify(&input)?;
    fs::write(input.join("decisions.jsonl"), b"{}").map_err(|e| e.to_string())?;
    assert!(snapshot.verify(&input).is_err());
    fs::remove_file(input.join("mcp.jsonl")).map_err(|e| e.to_string())?;
    std::os::unix::fs::symlink(input.join("result.json"), input.join("mcp.jsonl"))
        .map_err(|e| e.to_string())?;
    assert!(Snapshot::read(&input).is_err());
    let alias = root("snapshot-alias");
    std::os::unix::fs::symlink(&input, &alias).map_err(|e| e.to_string())?;
    assert!(Snapshot::read(&alias).is_err());
    fs::remove_file(alias).map_err(|e| e.to_string())?;
    fs::remove_dir_all(input).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn aggregate_budget_rejects_before_reading_next_file() -> Result<(), String> {
    let input = root("snapshot-budget");
    write_fixture(&input, false)?;
    for name in [
        "manifest.json",
        "result.json",
        "trajectory.jsonl",
        "decisions.jsonl",
    ] {
        fs::File::create(input.join(name))
            .and_then(|f| f.set_len(16 * 1024 * 1024))
            .map_err(|e| e.to_string())?;
    }
    // Four admitted files exhaust the aggregate budget. The next nonempty file
    // must fail the metadata bound before allocating or reading its contents.
    fs::write(input.join("mcp.jsonl"), b"{}").map_err(|e| e.to_string())?;
    assert!(matches!(Snapshot::read(&input), Err(e) if e == "source_file_limit_or_type"));
    fs::remove_dir_all(input).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn optional_accounting_and_partial_prefix_pass_oracle() -> Result<(), String> {
    let input = root("partial-optional-review");
    write_fixture(&input, false)?;
    fs::remove_file(input.join("provider-accounting.jsonl")).map_err(|e| e.to_string())?;
    let mut trajectory = fs::read(input.join("trajectory.jsonl")).map_err(|e| e.to_string())?;
    trajectory.extend_from_slice(b"{\"event\":");
    fs::write(input.join("trajectory.jsonl"), trajectory).map_err(|e| e.to_string())?;
    let output = root("partial-optional-review.zip");
    let report = export_directory(&input, &output)?;
    assert_eq!(report.emitted_events, 7);
    assert_eq!(report.emitted_accounting, 0);
    if let Some(validator) = std::env::var_os("STS2_RECORDED_RUN_VALIDATOR") {
        let status = std::process::Command::new("node")
            .arg(validator)
            .arg(&output)
            .status()
            .map_err(|e| e.to_string())?;
        assert!(status.success());
    }
    fs::remove_file(output).map_err(|e| e.to_string())?;
    fs::remove_dir_all(input).map_err(|e| e.to_string())?;
    Ok(())
}
