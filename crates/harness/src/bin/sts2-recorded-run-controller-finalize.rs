// SPDX-License-Identifier: MIT

//! Run a future external controller, then export only its finalized direct-child recording.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const MARKER: &[u8] = b"Controller running: ";
const MAX_LINE: usize = 65_536;
const MAX_OUTPUT: usize = 1_048_576;

fn main() {
    match parse(std::env::args_os().skip(1).collect()).and_then(|args| run(&args)) {
        Ok(code) => std::process::exit(code),
        Err(code) => std::process::exit(code),
    }
}

struct Args {
    runs_root: PathBuf,
    bundle_dir: PathBuf,
    exporter: PathBuf,
    command: Vec<OsString>,
}

fn parse(args: Vec<OsString>) -> Result<Args, i32> {
    let mut values = args.into_iter();
    let (Some(root), Some(dir), Some(exporter), Some(separator)) =
        (values.next(), values.next(), values.next(), values.next())
    else {
        return Err(64);
    };
    if separator != "--" {
        return Err(64);
    }
    let command = values.collect::<Vec<_>>();
    if command.is_empty() {
        return Err(64);
    }
    Ok(Args {
        runs_root: PathBuf::from(root),
        bundle_dir: PathBuf::from(dir),
        exporter: PathBuf::from(exporter),
        command,
    })
}

fn run(args: &Args) -> Result<i32, i32> {
    let root = args.runs_root.canonicalize().map_err(|_| 65)?;
    if !root.is_dir()
        || args
            .exporter
            .symlink_metadata()
            .map_err(|_| 65)?
            .file_type()
            .is_symlink()
    {
        return Err(65);
    }
    let exporter = args.exporter.canonicalize().map_err(|_| 65)?;
    let mut child = Command::new(&args.command[0])
        .args(&args.command[1..])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| 65)?;
    let stdout = child.stdout.take().ok_or(65)?;
    let marker_result = drain_markers(stdout);
    if marker_result.is_err() {
        let _ = child.kill();
    }
    let controller = child.wait().map_err(|_| 65)?;
    let markers = marker_result?;
    if markers.len() != 1 {
        return Err(65);
    }
    let source_input = &markers[0];
    if source_input
        .symlink_metadata()
        .map_err(|_| 65)?
        .file_type()
        .is_symlink()
    {
        return Err(65);
    }
    let source = source_input.canonicalize().map_err(|_| 65)?;
    if source.parent() != Some(root.as_path()) || !source.is_dir() {
        return Err(65);
    }
    let result = source.join("result.json");
    let result_meta = result.symlink_metadata().map_err(|_| 65)?;
    if result_meta.file_type().is_symlink() || !result_meta.is_file() {
        return Err(65);
    }
    std::fs::create_dir_all(&args.bundle_dir).map_err(|_| 65)?;
    let destination = args.bundle_dir.canonicalize().map_err(|_| 65)?;
    if destination.starts_with(&root) {
        return Err(65);
    }
    let output = destination.join(format!(
        "{}.recorded-run.zip",
        source.file_name().ok_or(65)?.to_string_lossy()
    ));
    if output.symlink_metadata().is_ok() {
        return Err(65);
    }
    let exported = Command::new(exporter)
        .args(["finalize", source.to_str().ok_or(65)?, "--output"])
        .arg(&output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| 65)?;
    let output_meta = output.symlink_metadata().map_err(|_| 65)?;
    if !exported.success() || output_meta.file_type().is_symlink() || !output_meta.is_file() {
        return Err(65);
    }
    Ok(controller.code().unwrap_or(128))
}

fn drain_markers(mut stdout: impl Read) -> Result<Vec<PathBuf>, i32> {
    let mut buffer = [0u8; 8192];
    let mut line = Vec::new();
    let mut markers = Vec::new();
    let mut total = 0usize;
    let mut invalid = false;
    loop {
        let read = stdout.read(&mut buffer).map_err(|_| 65)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            total = total.saturating_add(1);
            if total > MAX_OUTPUT {
                invalid = true;
            }
            if invalid {
                continue;
            }
            if *byte == b'\n' {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if let Some(path) = line.strip_prefix(MARKER) {
                    match std::str::from_utf8(path) {
                        Ok(path) => markers.push(PathBuf::from(path)),
                        Err(_) => invalid = true,
                    }
                }
                line.clear();
            } else if line.len() == MAX_LINE {
                invalid = true;
            } else {
                line.push(*byte);
            }
        }
    }
    if invalid || !line.is_empty() {
        return Err(65);
    }
    Ok(markers)
}
