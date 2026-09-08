// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use std::fs;
use std::time::{Duration, Instant};
use sts2_harness::{
    ExecutionCancellation, ExoError, ExoProcessConfig, ExoProcessTransport, ExoTransport,
    ExoTransportError,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn cancelled_execution_rejects_before_launch_and_is_not_a_timeout() -> TestResult {
    let cancellation = ExecutionCancellation::default();
    cancellation.cancel();
    let config = ExoProcessConfig::new("/nonexistent-provider", Vec::new(), None, Vec::new())?;
    let mut transport = ExoProcessTransport::new(config).with_cancellation(cancellation.clone());
    assert_eq!(
        transport.exchange(b"request", 512, 30_000),
        Err(ExoTransportError::Cancelled)
    );
    assert!(cancellation.is_cancelled());
    assert_eq!(
        ExoError::from(ExoTransportError::Cancelled),
        ExoError::Cancelled
    );
    Ok(())
}

#[test]
fn cancellation_interrupts_pending_response_and_reaps_direct_child() -> TestResult {
    cancel_active_exchange(b"request")
}

#[test]
fn cancellation_interrupts_full_stdin_pipe_and_reaps_direct_child() -> TestResult {
    cancel_active_exchange(&vec![b'x'; 1024 * 1024])
}

fn cancel_active_exchange(request: &[u8]) -> TestResult {
    let marker = std::env::temp_dir().join(format!("exo-cancel-{}", uuid::Uuid::new_v4()));
    let config = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            "-c".into(),
            "printf '%s' \"$$\" >\"$1\"; exec /bin/sleep 30".into(),
            "fixture".into(),
            marker.to_string_lossy().into_owned(),
        ],
        None,
        Vec::new(),
    )?;
    let cancellation = ExecutionCancellation::default();
    let mut transport = ExoProcessTransport::new(config).with_cancellation(cancellation.clone());
    let (pid, result, elapsed) = std::thread::scope(|scope| {
        let exchange = scope.spawn(|| transport.exchange(request, 512, 30_000));
        let deadline = Instant::now() + Duration::from_secs(3);
        let pid = loop {
            if let Ok(text) = fs::read_to_string(&marker)
                && let Ok(pid) = text.parse::<u32>()
            {
                break Some(pid);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let started = Instant::now();
        cancellation.cancel();
        (pid, exchange.join(), started.elapsed())
    });
    if marker.exists() {
        fs::remove_file(&marker)?;
    }
    let pid = pid.ok_or("synthetic provider did not confirm startup")?;
    assert_eq!(
        result.map_err(|_| "exchange thread failed")?,
        Err(ExoTransportError::Cancelled)
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "cancellation took {elapsed:?}"
    );
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert_eq!(
        transport.exchange(b"another request", 512, 30_000),
        Err(ExoTransportError::Cancelled)
    );
    Ok(())
}
