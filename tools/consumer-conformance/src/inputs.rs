// SPDX-License-Identifier: MIT

use crate::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::Command;

pub fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("consumer input exceeds one MiB".into());
    }
    Ok(bytes)
}

pub fn json_file(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&read_bounded(path)?)?)
}

pub fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex(&hash.finalize()))
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    if !output.status.success() {
        return Err("git identity command failed".into());
    }
    Ok(output.stdout)
}

pub fn identity(root: &Path) -> Result<Value> {
    let head = String::from_utf8(git(root, &["rev-parse", "HEAD"])?)?;
    let status = git(root, &["status", "--porcelain", "--untracked-files=all"])?;
    let diff = git(root, &["diff", "HEAD", "--binary"])?;
    // Include source bytes for untracked inputs; target/ remains ignored build output.
    let untracked = git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut inputs = Vec::new();
    for name in untracked
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = std::str::from_utf8(name)?;
        inputs.push(json!({"path": name, "sha256": hash_file(&root.join(name))?}));
    }
    let lockfile = if root.join("Cargo.lock").is_file() {
        "Cargo.lock"
    } else {
        "package-lock.json"
    };
    Ok(json!({
        "revision": head.trim(),
        "dirty": !status.is_empty(),
        "tracked_diff_sha256": digest(&diff),
        "untracked_inputs": inputs,
        "lockfile": lockfile,
        "lockfile_sha256": hash_file(&root.join(lockfile))?
    }))
}

pub fn consumer<'a>(matrix: &'a Value, repository: &str) -> Result<&'a Value> {
    let entries = matrix["consumers"].as_array().ok_or("missing consumers")?;
    let matches = entries
        .iter()
        .filter(|entry| entry["repository"] == repository)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err("consumer must be recorded exactly once".into());
    }
    Ok(matches[0])
}

pub fn verify_consumer(root: &Path, pin: &Value) -> Result<Value> {
    let identity = identity(root)?;
    if identity["revision"] != pin["revision"] || identity["dirty"] != false {
        return Err("consumer checkout is dirty or differs from its exact matrix pin".into());
    }
    let artifacts = pin["artifacts"]
        .as_array()
        .ok_or("missing artifact inventory")?;
    if artifacts.is_empty() {
        return Err("empty consumer artifact inventory".into());
    }
    for artifact in artifacts {
        let path = artifact["path"].as_str().ok_or("invalid artifact path")?;
        if Path::new(path).is_absolute()
            || Path::new(path)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err("artifact path escapes consumer root".into());
        }
        if artifact["sha256"] != hash_file(&root.join(path))? {
            return Err("consumer artifact digest drift".into());
        }
    }
    Ok(identity)
}

pub fn equal_fixture(candidate: &[u8], consumer: &[u8]) -> Result<()> {
    if candidate != consumer {
        return Err(
            "candidate producer fixture differs from consumer: coordinated migration required"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_candidate_bytes_fail_before_consumer_tests() {
        assert!(equal_fixture(b"trusted", b"trusted").is_ok());
        assert!(equal_fixture(b"tampered", b"trusted").is_err());
        assert!(equal_fixture(b"trusted\n", b"trusted").is_err());
    }

    #[test]
    fn unknown_or_duplicate_consumer_is_not_selected() {
        let matrix = json!({"consumers": [{"repository":"console"},{"repository":"console"}]});
        assert!(consumer(&matrix, "console").is_err());
        assert!(consumer(&matrix, "unknown").is_err());
    }
}
