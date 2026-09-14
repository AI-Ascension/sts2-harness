// SPDX-License-Identifier: MIT

mod candidate;
mod inputs;

use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let args = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: harness-consumer-conformance HARNESS CONSOLE STUDIO".into());
    }
    let harness = args[0].canonicalize()?;
    let console = args[1].canonicalize()?;
    let studio = args[2].canonicalize()?;
    let output = harness.join("target/consumer-conformance");
    fs::create_dir_all(&output)?;
    let matrix = inputs::json_file(&harness.join("contracts/effective-limits-pins.json"))?;
    let console_pin = inputs::consumer(&matrix, "AI-Ascension/ascension-context-console")?;
    let studio_pin = inputs::consumer(&matrix, "AI-Ascension/ascension-workflow-studio")?;
    let console_identity = inputs::verify_consumer(&console, console_pin)?;
    let studio_identity = inputs::verify_consumer(&studio, studio_pin)?;
    let candidate_identity = inputs::identity(&harness)?;
    let generated = candidate::generate(&harness, &console, &output)?;
    let golden = inputs::read_bounded(&console.join("fixtures/effective-limits/producer.json"))?;
    let studio_golden =
        inputs::read_bounded(&studio.join("contracts/accepted/effective-limits/producer.json"))?;
    inputs::equal_fixture(&generated.bytes, &golden)?;
    inputs::equal_fixture(&generated.bytes, &studio_golden)?;
    if inputs::identity(&harness)? != candidate_identity
        || inputs::verify_consumer(&console, console_pin)? != console_identity
        || inputs::verify_consumer(&studio, studio_pin)? != studio_identity
    {
        return Err("source identity changed during candidate generation".into());
    }
    let mut evidence = generator_evidence(&harness, &console, &output, &generated, &golden)?;
    evidence["candidate"] = candidate_identity;
    evidence["console"] = console_identity;
    evidence["studio"] = studio_identity;
    fs::write(
        output.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    println!("Candidate producer bytes equal both exact consumer fixtures; provenance recorded.");
    Ok(())
}

fn generator_evidence(
    harness: &Path,
    console: &Path,
    output: &Path,
    generated: &candidate::Generated,
    golden: &[u8],
) -> Result<Value> {
    let golden_value: Value = serde_json::from_slice(golden)?;
    Ok(json!({
        "schema": "ascension.harness.consumer-conformance-evidence.v1",
        "result": "candidate_generator_and_fixture_equality_passed",
        "evidence": "synthetic producer-library only; consumer tests are separate required CI steps",
        "verifier": {
            "version": env!("CARGO_PKG_VERSION"),
            "manifest_sha256": inputs::hash_file(&harness.join("tools/consumer-conformance/Cargo.toml"))?,
            "lockfile_sha256": inputs::hash_file(&harness.join("tools/consumer-conformance/Cargo.lock"))?,
            "binary_sha256": inputs::hash_file(&std::env::current_exe()?)?
        },
        "compiler": generated.compiler,
        "candidate_link_artifacts": generated.artifacts,
        "generator_path": "tools/effective-limit-fixtures/src/main.rs",
        "generator_sha256": inputs::hash_file(&console.join("tools/effective-limit-fixtures/src/main.rs"))?,
        "generator_binary_sha256": inputs::hash_file(&output.join("producer-generator"))?,
        "candidate_fixture_sha256": inputs::digest(&generated.bytes),
        "golden_origin_revision": golden_value["producer_revision"],
        "origin_note": "The unchanged generator embeds the golden origin. Actual executed producer source is candidate above, never inferred from that embedded field.",
        "consumer_source_modified": false
    }))
}
