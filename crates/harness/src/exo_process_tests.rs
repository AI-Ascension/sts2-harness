// SPDX-License-Identifier: MIT

use super::*;
use std::error::Error;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// The exchange futures never leave the calling thread, so a current-thread runtime is enough.
fn current_thread() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

#[test]
fn stderr_tail_keeps_only_the_last_bounded_bytes() -> Result<(), Box<dyn Error>> {
    let runtime = current_thread()?;
    let stream = b"0123456789abcdef".to_vec();
    let tail = runtime.block_on(read_tail(&stream[..], 6));
    assert_eq!(tail, b"abcdef".to_vec());
    Ok(())
}

#[test]
fn stderr_tail_keeps_a_stream_shorter_than_the_bound_whole() -> Result<(), Box<dyn Error>> {
    let runtime = current_thread()?;
    let stream = b"powershell.exe is not recognized".to_vec();
    let tail = runtime.block_on(read_tail(&stream[..], MAX_CHILD_STDERR_BYTES));
    assert_eq!(tail, stream);
    Ok(())
}

#[test]
fn stderr_tail_treats_an_empty_stream_as_no_diagnostic() -> Result<(), Box<dyn Error>> {
    let runtime = current_thread()?;
    let empty: Vec<u8> = Vec::new();
    let tail = runtime.block_on(read_tail(&empty[..], MAX_CHILD_STDERR_BYTES));
    assert!(tail.is_empty(), "an empty stderr must stay empty: {tail:?}");
    Ok(())
}

#[test]
fn stderr_tail_is_bounded_and_never_errors_on_a_verbose_child() -> Result<(), Box<dyn Error>> {
    let runtime = current_thread()?;
    // Far more than a pipe buffer, and more than any caller would want to print.
    let noisy = vec![b'x'; 512 * 1024];
    let tail = runtime.block_on(read_tail(&noisy[..], 128));
    assert_eq!(tail.len(), 128);
    assert!(tail.iter().all(|byte| *byte == b'x'));
    Ok(())
}

#[cfg(unix)]
#[test]
fn child_failure_diagnostic_names_the_status_and_the_tail() {
    let status = ExitStatus::from_raw(1 << 8);
    let diagnostic = child_failure_diagnostic(&status, b"powershell.exe is not recognized\n");
    assert!(
        diagnostic.contains("exit status: 1"),
        "the exit status must be named: {diagnostic}"
    );
    assert!(
        diagnostic.contains("powershell.exe is not recognized"),
        "the child's own words must survive: {diagnostic}"
    );
}

#[cfg(unix)]
#[test]
fn child_failure_diagnostic_says_so_when_stderr_is_empty() {
    let status = ExitStatus::from_raw(1 << 8);
    let diagnostic = child_failure_diagnostic(&status, b" \n\t");
    assert!(
        diagnostic.contains("wrote no stderr"),
        "a silent child must be distinguishable: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("stderr tail"),
        "a silent child has no tail to print: {diagnostic}"
    );
}

#[test]
fn start_failure_diagnostic_names_the_operating_system_error() {
    let error = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file or directory");
    let diagnostic = start_failure_diagnostic(&error);
    assert!(
        diagnostic.contains("no such file or directory"),
        "the start failure must name its cause: {diagnostic}"
    );
}
