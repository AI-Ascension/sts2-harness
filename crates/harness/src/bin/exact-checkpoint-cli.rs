// SPDX-License-Identifier: MIT

//! Trusted local commands over a stored exact-checkpoint artifact root.
//!
//! Commands operate only on the local artifact store: `list` enumerates published manifests,
//! `inspect` prints the privileged manifest summary, and `verify` checks integrity and declared
//! contracts without restoring anything. Restore and compare need a live runtime and are rejected
//! here rather than approximated.

use std::path::PathBuf;
use std::process::ExitCode;

use sts2_harness::{
    ExactArtifactStore, ExactAssurance, ExactCheckpointId, ExactCheckpointReference,
    ExactStateDigest, VerificationOutcome, verify_checkpoint,
};

const USAGE: &str = "usage: exact-checkpoint-cli <list|inspect|verify> --root DIR [options]\n\
  inspect --checkpoint ID\n\
  verify --checkpoint ID --state-digest DIGEST --compatibility DIGEST --coverage DIGEST\n\
         [--boundary-kind KIND] [--boundary-phase PHASE]";

enum Failure {
    Usage(String),
    Domain(String),
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
        Err(Failure::Domain(message)) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<String>) -> Result<(), Failure> {
    let mut arguments = arguments.into_iter();
    let command = arguments
        .next()
        .ok_or_else(|| Failure::Usage(USAGE.to_owned()))?;
    let options = parse(arguments)?;
    let root = options
        .root
        .clone()
        .ok_or_else(|| Failure::Usage(USAGE.to_owned()))?;
    let store = ExactArtifactStore::new(root);
    match command.as_str() {
        "list" => list(&store),
        "inspect" => inspect(&store, &options),
        "verify" => verify(&store, &options),
        "restore" | "compare" => Err(Failure::Domain(
            "restore and compare require a live runtime and are not available in this build"
                .to_owned(),
        )),
        other => Err(Failure::Usage(format!("unknown command {other}\n{USAGE}"))),
    }
}

fn list(store: &ExactArtifactStore) -> Result<(), Failure> {
    for identifier in store
        .stored_manifests()
        .map_err(|error| Failure::Domain(error.to_string()))?
    {
        println!("{}", identifier.as_str());
    }
    Ok(())
}

fn inspect(store: &ExactArtifactStore, options: &Options) -> Result<(), Failure> {
    let identifier = checkpoint(options)?;
    let bytes = store
        .read_manifest(&identifier)
        .map_err(|error| Failure::Domain(error.to_string()))?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| Failure::Domain("manifest is not readable JSON".to_owned()))?;
    let summary = serde_json::json!({
        "checkpoint": identifier.as_str(),
        "manifest_bytes": bytes.len(),
        "exact_state_digest": manifest.get("exact_state_digest").cloned(),
        "compatibility_digest": manifest.get("compatibility_digest").cloned(),
        "coverage_contract_digest": manifest.get("coverage_contract_digest").cloned(),
        "blobs": store
            .stored_blobs()
            .map_err(|error| Failure::Domain(error.to_string()))?
            .iter()
            .map(|digest| digest.as_str().to_owned())
            .collect::<Vec<_>>(),
    });
    println!("{summary}");
    Ok(())
}

fn verify(store: &ExactArtifactStore, options: &Options) -> Result<(), Failure> {
    let identifier = checkpoint(options)?;
    let state = options
        .state_digest
        .as_deref()
        .ok_or_else(|| Failure::Usage(USAGE.to_owned()))?;
    let compatibility = options
        .compatibility
        .as_deref()
        .ok_or_else(|| Failure::Usage(USAGE.to_owned()))?;
    let coverage = options
        .coverage
        .as_deref()
        .ok_or_else(|| Failure::Usage(USAGE.to_owned()))?;
    let manifest: serde_json::Value = serde_json::from_slice(
        &store
            .read_manifest(&identifier)
            .map_err(|error| Failure::Domain(error.to_string()))?,
    )
    .map_err(|_| Failure::Domain("malformed_manifest".to_owned()))?;
    let boundary = |field: &str| {
        manifest["boundary"][field]
            .as_str()
            .unwrap_or("unspecified")
            .to_owned()
    };
    let reference = ExactCheckpointReference {
        exact_state_digest: ExactStateDigest::parse(state)
            .map_err(|error| Failure::Usage(error.to_string()))?,
        exact_checkpoint_id: identifier,
        boundary_kind: options
            .boundary_kind
            .clone()
            .unwrap_or_else(|| boundary("kind")),
        boundary_phase: options
            .boundary_phase
            .clone()
            .unwrap_or_else(|| boundary("phase")),
        assurance: ExactAssurance::CaptureOnly,
    };
    match verify_checkpoint(store, &reference, compatibility, coverage) {
        VerificationOutcome::Verified(evidence) => {
            println!(
                "{{\"status\":\"verified\",\"level\":\"{}\"}}",
                evidence.highest_level()
            );
            Ok(())
        }
        VerificationOutcome::Rejected(failure) => Err(Failure::Domain(format!(
            "{{\"status\":\"rejected\",\"reason\":\"{}\"}}",
            failure.as_str()
        ))),
    }
}

fn checkpoint(options: &Options) -> Result<ExactCheckpointId, Failure> {
    let value = options
        .checkpoint
        .as_deref()
        .ok_or_else(|| Failure::Usage(USAGE.to_owned()))?;
    ExactCheckpointId::parse(value).map_err(|error| Failure::Usage(error.to_string()))
}

#[derive(Default)]
struct Options {
    root: Option<PathBuf>,
    checkpoint: Option<String>,
    state_digest: Option<String>,
    compatibility: Option<String>,
    coverage: Option<String>,
    boundary_kind: Option<String>,
    boundary_phase: Option<String>,
}

fn parse(arguments: impl Iterator<Item = String>) -> Result<Options, Failure> {
    let mut options = Options::default();
    let mut arguments = arguments.peekable();
    while let Some(flag) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| Failure::Usage(format!("{flag} requires a value\n{USAGE}")))?;
        match flag.as_str() {
            "--root" => options.root = Some(PathBuf::from(value)),
            "--checkpoint" => options.checkpoint = Some(value),
            "--state-digest" => options.state_digest = Some(value),
            "--compatibility" => options.compatibility = Some(value),
            "--coverage" => options.coverage = Some(value),
            "--boundary-kind" => options.boundary_kind = Some(value),
            "--boundary-phase" => options.boundary_phase = Some(value),
            other => return Err(Failure::Usage(format!("unknown flag {other}\n{USAGE}"))),
        }
    }
    Ok(options)
}
