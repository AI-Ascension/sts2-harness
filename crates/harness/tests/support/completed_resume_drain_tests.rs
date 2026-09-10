// SPDX-License-Identifier: MIT

//! Adversarial readers exercise a single drain call independently of process timing.
use super::{CaptureStream, MAX_DRAIN_READS, drain_stream};
use std::io::{self, ErrorKind, Read};

struct ForeverBytes {
    calls: usize,
}

impl Read for ForeverBytes {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        buffer[0] = b'x';
        Ok(1)
    }
}

struct ForeverInterrupted {
    calls: usize,
}

impl Read for ForeverInterrupted {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        Err(io::Error::from(ErrorKind::Interrupted))
    }
}

#[test]
fn drain_stream_bounds_a_continuous_reader_to_one_drain_call() -> Result<(), String> {
    let mut stream = CaptureStream::new(ForeverBytes { calls: 0 }, "test");
    stream.capture = false;
    drain_stream(&mut stream)?;
    assert!(stream.open);
    assert_eq!(stream.reader.calls, MAX_DRAIN_READS);
    Ok(())
}

#[test]
fn drain_stream_bounds_repeated_interrupted_reads() -> Result<(), String> {
    let mut stream = CaptureStream::new(ForeverInterrupted { calls: 0 }, "test");
    drain_stream(&mut stream)?;
    assert!(stream.open);
    assert_eq!(stream.reader.calls, MAX_DRAIN_READS);
    Ok(())
}
