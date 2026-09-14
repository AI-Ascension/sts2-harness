// SPDX-License-Identifier: MIT

use crate::{Result, inputs};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Generated {
    pub bytes: Vec<u8>,
    pub compiler: String,
    pub artifacts: Value,
}

pub fn generate(harness: &Path, console: &Path, output: &Path) -> Result<Generated> {
    let compiler = compiler_version(harness)?;
    let libraries = build(harness, output)?;
    let executable = compile_generator(harness, console, output, &libraries)?;
    let generated = output.join("candidate-output.json");
    let status = Command::new(&executable)
        .current_dir(harness)
        .stdin(Stdio::null())
        .stdout(File::create(&generated)?)
        .status()?;
    if !status.success() {
        return Err("candidate-linked generator failed".into());
    }
    Ok(Generated {
        bytes: inputs::read_bounded(&generated)?,
        compiler,
        artifacts: json!([
            artifact_evidence(harness, &libraries.owner)?,
            artifact_evidence(harness, &libraries.json)?
        ]),
    })
}

fn compiler_version(harness: &Path) -> Result<String> {
    let version = Command::new("rustc")
        .current_dir(harness)
        .arg("-Vv")
        .output()?;
    let compiler = String::from_utf8(version.stdout)?;
    if !version.status.success() || !compiler.lines().any(|line| line == "release: 1.97.1") {
        return Err("candidate generator requires pinned Rust 1.97.1".into());
    }
    Ok(compiler)
}

struct Libraries {
    target: PathBuf,
    owner: PathBuf,
    json: PathBuf,
}

fn build(harness: &Path, output: &Path) -> Result<Libraries> {
    let messages = output.join("cargo-artifacts.jsonl");
    let target = output.join("candidate-build");
    let status = Command::new("cargo")
        .current_dir(harness)
        .args([
            "build",
            "--locked",
            "--package",
            "sts2-harness",
            "--lib",
            "--message-format=json",
        ])
        .arg("--target-dir")
        .arg(&target)
        .stdout(File::create(&messages)?)
        .stderr(Stdio::inherit())
        .status()?;
    if !status.success() {
        return Err("locked candidate library build failed".into());
    }
    let artifacts = read_artifacts(&messages)?;
    let owner = select_artifact(
        &artifacts,
        "sts2_harness",
        Some(&harness.join("crates/harness/Cargo.toml")),
    )?;
    let json = select_artifact(&artifacts, "serde_json", None)?;
    Ok(Libraries {
        target,
        owner,
        json,
    })
}

fn compile_generator(
    harness: &Path,
    console: &Path,
    output: &Path,
    libraries: &Libraries,
) -> Result<PathBuf> {
    let executable = output.join("producer-generator");
    let status = Command::new("rustc")
        .current_dir(harness)
        .arg(console.join("tools/effective-limit-fixtures/src/main.rs"))
        .args([
            "--edition",
            "2024",
            "--crate-name",
            "candidate_consumer_generator",
        ])
        .arg("--extern")
        .arg(format!("sts2_harness={}", libraries.owner.display()))
        .arg("--extern")
        .arg(format!("serde_json={}", libraries.json.display()))
        .arg("-L")
        .arg(format!(
            "dependency={}",
            libraries.target.join("debug/deps").display()
        ))
        .arg("-o")
        .arg(&executable)
        .status()?;
    if !status.success() {
        return Err("unchanged consumer generator did not compile against candidate".into());
    }
    Ok(executable)
}

fn artifact_evidence(root: &Path, artifact: &Path) -> Result<Value> {
    Ok(json!({
        "path": artifact.strip_prefix(root)?.to_string_lossy(),
        "sha256": inputs::hash_file(artifact)?
    }))
}

fn read_artifacts(path: &Path) -> Result<Vec<Value>> {
    if fs::metadata(path)?.len() > 67_108_864 {
        return Err("candidate compiler messages exceed 64 MiB".into());
    }
    let mut result = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let value: Value = serde_json::from_str(&line?)?;
        if value["reason"] == "compiler-artifact" {
            result.push(value);
        }
    }
    Ok(result)
}

fn select_artifact(artifacts: &[Value], name: &str, manifest: Option<&Path>) -> Result<PathBuf> {
    let mut paths = BTreeSet::new();
    for artifact in artifacts {
        if artifact["target"]["name"] != name || artifact["profile"]["test"] != false {
            continue;
        }
        if let Some(manifest) = manifest
            && artifact["manifest_path"].as_str() != manifest.to_str()
        {
            continue;
        }
        for filename in artifact["filenames"]
            .as_array()
            .ok_or("missing compiler filenames")?
        {
            let path = PathBuf::from(filename.as_str().ok_or("invalid compiler filename")?);
            if path
                .extension()
                .is_some_and(|extension| extension == "rlib")
            {
                paths.insert(path);
            }
        }
    }
    if paths.len() != 1 {
        return Err("candidate link artifact missing or ambiguous".into());
    }
    paths
        .into_iter()
        .next()
        .ok_or_else(|| "candidate artifact absent".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(manifest: &str, path: &str) -> Value {
        json!({"target":{"name":"sts2_harness"}, "profile":{"test":false},
            "manifest_path":manifest, "filenames":[path]})
    }

    #[test]
    fn candidate_link_selection_rejects_wrong_manifest_test_profile_and_ambiguity() {
        let manifest = Path::new("/candidate/crates/harness/Cargo.toml");
        let good = artifact(
            "/candidate/crates/harness/Cargo.toml",
            "/build/libowner.rlib",
        );
        assert!(
            select_artifact(std::slice::from_ref(&good), "sts2_harness", Some(manifest)).is_ok()
        );
        let wrong = artifact("/old/crates/harness/Cargo.toml", "/old/libowner.rlib");
        assert!(select_artifact(&[wrong], "sts2_harness", Some(manifest)).is_err());
        let mut test = good.clone();
        test["profile"]["test"] = json!(true);
        assert!(select_artifact(&[test], "sts2_harness", Some(manifest)).is_err());
        let another = artifact(
            "/candidate/crates/harness/Cargo.toml",
            "/other/libowner.rlib",
        );
        assert!(select_artifact(&[good, another], "sts2_harness", Some(manifest)).is_err());
    }
}
