// SPDX-License-Identifier: MIT

use std::env;
use std::fs::File;
use std::io::Read;
use std::process::ExitCode;

const MAX_INPUT_BYTES: usize = 512 * 1024;
const MAX_REPORTED_LINES: usize = 128;

struct Manifest {
    text: String,
    quarantine_status: String,
}

#[derive(serde::Deserialize)]
struct ManifestStatus {
    quarantine: QuarantineStatus,
}

#[derive(serde::Deserialize)]
struct QuarantineStatus {
    status: String,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sts2-patch-diff: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let before_path = arguments.next().ok_or_else(usage)?;
    let after_path = arguments.next().ok_or_else(usage)?;
    if arguments.next().is_some() {
        return Err(usage());
    }
    let before = read_manifest(&before_path)?;
    let after = read_manifest(&after_path)?;
    let before_lines = before.text.lines().collect::<Vec<_>>();
    let after_lines = after.text.lines().collect::<Vec<_>>();
    let line_count = before_lines.len().max(after_lines.len());
    let mut changed = Vec::new();
    let mut changed_count = 0;
    for index in 0..line_count {
        if before_lines.get(index) != after_lines.get(index) {
            changed_count += 1;
            if changed.len() < MAX_REPORTED_LINES {
                changed.push(index + 1);
            }
        }
    }
    let output = format!(
        "{{\"tool\":\"sts2-patch-diff\",\"version\":\"0.1.0\",\"before\":{{\"path\":{},\"fingerprint\":\"{}\",\"quarantine_status\":{}}},\"after\":{{\"path\":{},\"fingerprint\":\"{}\",\"quarantine_status\":{}}},\"changed_line_count\":{},\"reported_line_numbers\":{},\"truncated\":{}}}\n",
        json_string(&before_path),
        fingerprint(&before.text),
        json_string(&before.quarantine_status),
        json_string(&after_path),
        fingerprint(&after.text),
        json_string(&after.quarantine_status),
        changed_count,
        json_array(&changed),
        changed_count > MAX_REPORTED_LINES,
    );
    print!("{output}");
    Ok(())
}

fn read_manifest(path: &str) -> Result<Manifest, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    read_bounded(file).map_err(|error| format!("{path}: {error}"))
}

fn read_bounded(reader: impl Read) -> Result<Manifest, String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read manifest: {error}"))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!("exceeds the {MAX_INPUT_BYTES}-byte input bound"));
    }
    let text = String::from_utf8(bytes).map_err(|_| String::from("manifest is not UTF-8"))?;
    let value: ManifestStatus = serde_json::from_str(&text)
        .map_err(|_| String::from("invalid JSON or missing/ambiguous quarantine.status"))?;
    let quarantine_status = value.quarantine.status;
    if !matches!(
        quarantine_status.as_str(),
        "quarantined" | "eligible" | "promoted" | "rejected"
    ) {
        return Err(String::from("manifest has no valid quarantine.status"));
    }
    Ok(Manifest {
        text,
        quarantine_status,
    })
}

fn fingerprint(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64-{hash:016x}")
}

fn json_string(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}

fn json_array(values: &[usize]) -> String {
    let mut output = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&value.to_string());
    }
    output.push(']');
    output
}

fn usage() -> String {
    String::from("usage: sts2-patch-diff <base-manifest.json> <candidate-manifest.json>")
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
