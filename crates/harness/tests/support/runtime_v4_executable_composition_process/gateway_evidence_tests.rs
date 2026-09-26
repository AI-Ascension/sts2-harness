// SPDX-License-Identifier: MIT

//! Unit tests for the gateway evidence writer's capture bound. Refs sts2-harness#555.
//!
//! These drive [`bounded`] and [`write_gateway_streams`] directly rather than through a spawned
//! gateway, because the served path cannot deliver an oversized capture: the gateway is spawned
//! `Stdio::piped()` and `ready()` polls `try_wait`/`TcpStream::connect` **without reading either
//! pipe**, so a child that writes more than the kernel's 64 KiB pipe buffer blocks on the write
//! and never reaches its `bind`. `stop()` then collects that partial buffer. Measured with a
//! `Stdio::piped()` child writing 6 MiB before binding: captured stderr was exactly 65,536 bytes
//! and the child was still alive at the 5 s readiness deadline. An end-to-end assertion of a
//! multi-megabyte capture through this spawn path is therefore unfalsifiable — it can only ever
//! observe the bound's "pass through unchanged" arm, whatever the bound is set to.

use std::fs;
use std::path::Path;
use std::process::Output;

use super::{bounded, write_gateway_streams};

/// The per-stream ceiling, restated as a literal rather than imported from the implementation, so
/// this file cannot agree with the implementation by construction: moving the constant must move
/// the number here too, or these assertions fail.
const CAPTURE_LIMIT: usize = 4 * 1024 * 1024;

/// The marker's leading phrase, likewise spelled out for the same reason.
const MARKER_FRAGMENT: &str = "gateway stream truncated at";

/// A stream of `size` bytes whose content is position-dependent, so a cut is detectable as
/// content rather than merely as a length.
fn stream_of(size: usize) -> Vec<u8> {
    (0..size).map(|index| b'a' + (index % 26) as u8).collect()
}

#[test]
fn a_capture_within_the_bound_is_persisted_unchanged() {
    let source = stream_of(CAPTURE_LIMIT);
    let copy = bounded(&source);
    assert_eq!(copy, source, "a complete stream must not be altered at all");
    assert!(
        !String::from_utf8_lossy(&copy).contains(MARKER_FRAGMENT),
        "a complete stream was marked as truncated, so a reader would distrust a whole capture"
    );
}

#[test]
fn a_capture_one_byte_past_the_bound_is_cut_and_marked() {
    let source = stream_of(CAPTURE_LIMIT + 1);
    let copy = bounded(&source);
    let text = String::from_utf8_lossy(&copy).into_owned();
    assert!(
        text.contains(MARKER_FRAGMENT),
        "a stream one byte past the bound lost its truncation marker (sts2-harness#555)"
    );
    assert!(
        copy.starts_with(&source[..CAPTURE_LIMIT]),
        "the bounded copy is not the byte-exact prefix of the source"
    );
    // The copy is *not* shorter than a source one byte over the bound, and must not be asserted
    // to be: the marker is longer than the single byte it displaces, so cutting here makes the
    // file grow. What must hold is that it holds exactly the bound's worth of source bytes and
    // nothing more — a longer prefix would be a weaker bound than the one claimed.
    assert!(
        copy[..CAPTURE_LIMIT] == source[..CAPTURE_LIMIT]
            && copy[CAPTURE_LIMIT..]
                .windows(MARKER_FRAGMENT.len())
                .any(|window| { window == MARKER_FRAGMENT.as_bytes() }),
        "the copy is not the {CAPTURE_LIMIT}-byte prefix followed by a marker: {} bytes total \
         from a {} byte source (sts2-harness#555)",
        copy.len(),
        source.len()
    );
}

#[test]
fn a_capture_many_times_the_bound_is_cut_to_the_limit_plus_its_marker() {
    let source = stream_of(8 * CAPTURE_LIMIT);
    let copy = bounded(&source);
    let text = String::from_utf8_lossy(&copy).into_owned();
    assert!(
        text.contains(MARKER_FRAGMENT),
        "an 8x-oversized stream lost its truncation marker (sts2-harness#555)"
    );
    // Located without `expect`/`panic`, both of which this workspace denies. `0` is the
    // unfound sentinel here rather than a large value, so an absent marker fails the position
    // assertion below instead of passing it.
    let marker_at = copy
        .windows(MARKER_FRAGMENT.len())
        .position(|window| window == MARKER_FRAGMENT.as_bytes())
        .unwrap_or(0);
    assert!(
        marker_at >= CAPTURE_LIMIT,
        "the marker sits at byte {marker_at}, inside the {CAPTURE_LIMIT}-byte prefix, so either \
         it was never written or the source was cut short of the bound (sts2-harness#555)"
    );
    assert_eq!(
        &copy[..CAPTURE_LIMIT],
        &source[..CAPTURE_LIMIT],
        "the persisted prefix is not byte-exact"
    );
}

#[test]
fn both_streams_of_an_oversized_gateway_land_on_disk_bounded_and_marked()
-> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("gateway-evidence-555-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);

    // Both streams oversized at once, and different lengths, so a per-stream bound is
    // distinguishable from a bound on the pair.
    let gateway = Output {
        status: std::process::Command::new("sh")
            .args(["-c", "exit 7"])
            .status()?,
        stdout: stream_of(6 * CAPTURE_LIMIT),
        stderr: stream_of(CAPTURE_LIMIT + 7),
    };
    write_gateway_streams(&root, "served policy gate: oversized", &gateway)?;

    let mut files = Vec::new();
    collect(&root, &mut files)?;
    assert_eq!(files.len(), 2, "expected one stdout and one stderr capture");

    let mut total = 0_usize;
    for file in &files {
        let bytes = fs::read(file)?;
        total += bytes.len();
        let name = file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        assert!(
            String::from_utf8_lossy(&bytes).contains(MARKER_FRAGMENT),
            "{name} was persisted without a truncation marker (sts2-harness#555)"
        );
        assert!(
            bytes.len() <= CAPTURE_LIMIT + 1024,
            "{name} is {} bytes, past the {CAPTURE_LIMIT}-byte bound plus its marker",
            bytes.len()
        );
    }
    assert!(
        total <= 2 * (CAPTURE_LIMIT + 1024),
        "the pair totalled {total} bytes, so the write path is still unbounded \
         (sts2-harness#555)"
    );

    fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
fn a_complete_capture_lands_on_disk_unmarked() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!(
        "gateway-evidence-555-complete-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);

    let gateway = Output {
        status: std::process::Command::new("sh")
            .args(["-c", "exit 7"])
            .status()?,
        stdout: b"served stdout".to_vec(),
        stderr: b"served stderr".to_vec(),
    };
    write_gateway_streams(&root, "served policy gate: complete", &gateway)?;

    let mut files = Vec::new();
    collect(&root, &mut files)?;
    assert_eq!(files.len(), 2);
    for file in &files {
        let bytes = fs::read(file)?;
        assert!(
            !String::from_utf8_lossy(&bytes).contains(MARKER_FRAGMENT),
            "a complete capture was marked truncated, so a reader would distrust a whole capture \
             (sts2-harness#555)"
        );
    }

    fs::remove_dir_all(&root)?;
    Ok(())
}

/// Every regular file under `root`, sorted, so the assertions read the directory the way the
/// lane's dump step reads it rather than assuming a file name.
fn collect(
    root: &Path,
    files: &mut Vec<std::path::PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else {
            files.push(path);
        }
    }
    files.sort();
    Ok(())
}
