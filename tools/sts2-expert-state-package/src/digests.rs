// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::integrity::{read_json, require};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(crate) fn validate_digests(root: &Path) -> Result<()> {
    let manifest = read_json(&root.join("data/build-manifest.json"))?;
    let Some(digests) = manifest["artifact_sha256"].as_object() else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "missing artifact digests").into());
    };
    let expected: BTreeSet<_> = [
        "states.json",
        "observations.json",
        "actions.json",
        "transitions.json",
        "information-importance.csv",
        "state-field-matrix.csv",
        "expert-use-matrix.csv",
        "source-ledger.csv",
        "unresolved-questions.csv",
        "claim-evidence-matrix.csv",
    ]
    .into_iter()
    .collect();
    require(
        digests.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected,
        "inventory digest coverage drift",
    )?;
    for (path, expected) in digests {
        require(
            Path::new(path).file_name().and_then(|name| name.to_str()) == Some(path),
            "unsafe artifact path",
        )?;
        let actual = format!(
            "{:x}",
            Sha256::digest(fs::read(root.join("data").join(path))?)
        );
        require(expected == &actual, "inventory artifact digest mismatch")?;
    }
    validate_supporting_digests(root, &manifest)?;
    let renderer = &manifest["generation"]["renderer"];
    let paths = renderer["source_code_paths"].as_array().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing renderer source paths")
    })?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut records = String::new();
    for path in paths {
        let path = path.as_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid renderer source path")
        })?;
        require(
            !Path::new(path).is_absolute()
                && !Path::new(path)
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir)),
            "unsafe renderer path",
        )?;
        records.push_str(&format!(
            "{:x}  {path}\n",
            Sha256::digest(fs::read(repo.join(path))?)
        ));
    }
    require(
        renderer["source_code_digest"] == format!("{:x}", Sha256::digest(records.as_bytes())),
        "renderer source digest mismatch",
    )?;
    Ok(())
}

fn validate_supporting_digests(root: &Path, manifest: &serde_json::Value) -> Result<()> {
    let expected: BTreeSet<_> = [
        "schemas/game-observation.schema.json",
        "schemas/legal-action.schema.json",
        "schemas/decision-record.schema.json",
        "schemas/state-transition.schema.json",
        "schemas/patch-manifest.schema.json",
        "fixtures/manifest.json",
        "fixtures/normal.jsonl",
        "fixtures/boundary.jsonl",
        "fixtures/adversarial.jsonl",
        "fixtures/recovery.jsonl",
        "fixtures/patch-regression.jsonl",
        "fixtures/schema/cases.json",
        "fixtures/schema/game-observation.json",
        "fixtures/schema/legal-action.json",
        "fixtures/schema/decision-record.json",
        "fixtures/schema/state-transition.json",
        "fixtures/schema/patch-manifest.json",
    ]
    .into_iter()
    .collect();
    let digests = manifest["supporting_artifact_sha256"]
        .as_object()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "missing schema and fixture digests",
            )
        })?;
    require(
        digests.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected,
        "schema and fixture digest coverage drift",
    )?;
    for (path, expected) in digests {
        let actual = format!("{:x}", Sha256::digest(fs::read(root.join(path))?));
        require(expected == &actual, "schema or fixture digest mismatch")?;
    }
    Ok(())
}
