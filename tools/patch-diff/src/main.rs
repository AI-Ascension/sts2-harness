// SPDX-License-Identifier: MIT

use std::env;
use std::fs;
use std::process::ExitCode;

const MAX_INPUT_BYTES: usize = 512 * 1024;
const MAX_REPORTED_LINES: usize = 128;

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
    let before_lines = before.lines().collect::<Vec<_>>();
    let after_lines = after.lines().collect::<Vec<_>>();
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
        fingerprint(&before),
        extracted_status(&before),
        json_string(&after_path),
        fingerprint(&after),
        extracted_status(&after),
        changed_count,
        json_array(&changed),
        changed_count > MAX_REPORTED_LINES,
    );
    print!("{output}");
    Ok(())
}

fn extracted_status(input: &str) -> String {
    let section = input
        .find("\"quarantine\"")
        .map(|offset| &input[offset..]);
    match section.and_then(|value| extract_string(value, "status")) {
        Some(status) => json_string(status),
        None => String::from("\"unknown\""),
    }
}

fn read_manifest(path: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!("{path} exceeds the {MAX_INPUT_BYTES}-byte input bound"));
    }
    String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8"))
}

fn fingerprint(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64-{hash:016x}")
}

fn extract_string<'a>(input: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("\"{key}\"");
    let start = input.find(&marker)? + marker.len();
    let value = input[start..].find(':').map(|offset| start + offset + 1)?;
    let quoted = input[value..].find('"').map(|offset| value + offset + 1)?;
    let end = input[quoted..].find('"').map(|offset| quoted + offset)?;
    Some(&input[quoted..end])
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => output.push('?'),
            character => output.push(character),
        }
    }
    output.push('"');
    output
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
