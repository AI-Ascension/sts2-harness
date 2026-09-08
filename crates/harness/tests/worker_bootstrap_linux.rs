// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]

use rustix::pipe::pipe;
use std::io::Write;
use std::time::{Duration, Instant};
use sts2_harness::worker_bootstrap::{BOOTSTRAP_MAGIC, WorkerBootstrap};
use sts2_harness::worker_bootstrap_linux::{BootstrapReadError, read_owned_pipe};

fn frame() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let json = include_bytes!("../../../protocol-artifact/worker-bootstrap-v1/valid/linux.json");
    let mut frame = BOOTSTRAP_MAGIC.to_vec();
    frame.extend_from_slice(&u32::try_from(json.len())?.to_be_bytes());
    frame.extend_from_slice(json);
    Ok(frame)
}

#[test]
fn valid_pipe_returns_without_eof_and_closes_owned_reader() -> Result<(), Box<dyn std::error::Error>>
{
    let (reader, writer) = pipe()?;
    let mut writer = std::fs::File::from(writer);
    writer.write_all(&frame()?)?;
    let bootstrap = read_owned_pipe(reader, Duration::from_secs(1))?;
    assert_eq!(bootstrap.component_id(), "harness");
    assert_eq!(
        writer.write(b"x").err().map(|e| e.kind()),
        Some(std::io::ErrorKind::BrokenPipe)
    );
    Ok(())
}

#[test]
fn silent_writer_is_deadline_bounded_and_reader_is_closed() -> Result<(), Box<dyn std::error::Error>>
{
    let (reader, writer) = pipe()?;
    let mut writer = std::fs::File::from(writer);
    let start = Instant::now();
    assert!(matches!(
        read_owned_pipe(reader, Duration::from_millis(25)),
        Err(BootstrapReadError::Deadline)
    ));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        writer.write(b"x").err().map(|e| e.kind()),
        Some(std::io::ErrorKind::BrokenPipe)
    );
    Ok(())
}

#[test]
fn partial_oversized_and_trailing_frames_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
    let valid = frame()?;
    let mut oversized = BOOTSTRAP_MAGIC.to_vec();
    oversized.extend_from_slice(&16_385_u32.to_be_bytes());
    let mut doubled = valid.clone();
    doubled.extend_from_slice(&valid);
    for bytes in [&valid[..10], oversized.as_slice(), doubled.as_slice()] {
        let (reader, writer) = pipe()?;
        let mut writer = std::fs::File::from(writer);
        writer.write_all(bytes)?;
        drop(writer);
        assert!(read_owned_pipe(reader, Duration::from_secs(1)).is_err());
    }
    let null = std::fs::File::open("/dev/null")?;
    assert!(matches!(
        read_owned_pipe(null.into(), Duration::from_secs(1)),
        Err(BootstrapReadError::Invalid)
    ));
    assert!(WorkerBootstrap::decode(&valid).is_ok());
    Ok(())
}
