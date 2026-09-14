// SPDX-License-Identifier: MIT

//! Offline fixture generation with a separate actual-source provenance record.

#[path = "context_control_catalog/fixtures.rs"]
mod fixtures;

use std::error::Error;
use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Result<Vec<u8>, Box<dyn Error>> {
    let output = Command::new("git").args(args).output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("git identity command failed").into());
    }
    Ok(output.stdout)
}

fn text(bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    Ok(std::str::from_utf8(bytes)?.trim().to_owned())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 || args[0] == args[1] {
        return Err("usage: context_control_catalog <fixture-output> <evidence-output>".into());
    }
    let before = git(&["status", "--porcelain=v1", "--untracked-files=all"])?;
    let revision = text(&git(&["rev-parse", "HEAD"])?)?;
    let generated = fixtures::bytes()?;
    let evidence = evidence(&revision, &before, &generated)?;
    if before != git(&["status", "--porcelain=v1", "--untracked-files=all"])?
        || revision != text(&git(&["rev-parse", "HEAD"])?)?
    {
        return Err("source identity changed while generating fixtures".into());
    }
    write(Path::new(&args[0]), &generated)?;
    write(Path::new(&args[1]), &serde_json::to_vec_pretty(&evidence)?)?;
    Ok(())
}

fn evidence(
    revision: &str,
    status: &[u8],
    generated: &[u8],
) -> Result<serde_json::Value, Box<dyn Error>> {
    let paths = [
        "Cargo.lock",
        "crates/harness/src/management/context_owner.rs",
        "crates/harness/src/management/context_owner_support.rs",
        "crates/harness/examples/context_control_catalog.rs",
        "crates/harness/examples/context_control_catalog/fixtures.rs",
    ];
    let mut sources = serde_json::Map::new();
    for path in paths {
        sources.insert(
            path.to_owned(),
            sts2_harness::sha256_hex(std::fs::read(path)?).into(),
        );
    }
    Ok(serde_json::json!({
        "schema": "ascension.context-control.catalog-generation-evidence.v1",
        "candidate_revision": revision,
        "dirty": !status.is_empty(),
        "status_sha256": sts2_harness::sha256_hex(status),
        "tracked_diff_sha256": sts2_harness::sha256_hex(git(&["diff", "HEAD", "--binary"])?),
        "source_sha256": sources,
        "generator_binary_sha256": sts2_harness::sha256_hex(std::fs::read(std::env::current_exe()?)?),
        "fixture_sha256": sts2_harness::sha256_hex(generated),
        "embedded_origin_revision": fixtures::ORIGIN,
        "result": "synthetic_descriptor_validation_only"
    }))
}
