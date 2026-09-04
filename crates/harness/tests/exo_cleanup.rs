// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};
use sts2_harness::{ExoProcessConfig, ExoProcessTransport, ExoTransport, ExoTransportError};

fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task")
        .expect("Linux task directory")
        .count()
}

// A separate integration-test executable isolates thread accounting from other process tests.
#[test]
fn inherited_pipes_cannot_leave_harness_workers_after_timeout() {
    let before = thread_count();
    let mut outcomes = Vec::new();
    let started = Instant::now();
    for _ in 0..4 {
        let config = ExoProcessConfig::new(
            "/bin/sh",
            vec![String::from("-c"), String::from("sleep 2 <&0 & exit 0")],
            None,
            Vec::new(),
        )
        .expect("finite descendant fixture");
        let mut transport = ExoProcessTransport::new(config);
        outcomes.push(transport.exchange(&vec![b'x'; 1024 * 1024], 512, 30));
    }
    let elapsed = started.elapsed();
    let after = thread_count();
    // Let the deliberately independent finite fixture descendants exit before assertions.
    // This is fixture cleanup, not a retry loop or the timeout oracle.
    std::thread::sleep(Duration::from_millis(2200));
    assert!(
        outcomes
            .iter()
            .all(|result| *result == Err(ExoTransportError::Timeout))
    );
    assert!(elapsed < Duration::from_secs(1));
    assert_eq!(
        before, after,
        "every supervisor and pipe task must be gone on return"
    );
}
