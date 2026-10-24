// SPDX-License-Identifier: MIT

use std::error::Error;
use std::io::{self, Write};
use std::process::{Child, Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) type TestResult<T> = Result<T, Box<dyn Error>>;

pub(super) fn failure(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::other(message.into()))
}

fn combined(context: &str, first: &dyn Error, second: &dyn Error) -> Box<dyn Error> {
    failure(format!("{context}: {first}; {second}"))
}

fn cleanup_child(child: &mut Child) -> io::Result<()> {
    let kill_result = child.kill();
    let wait_result = child.wait();
    match (kill_result, wait_result) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(error), Ok(_)) if error.kind() == io::ErrorKind::InvalidInput => Ok(()),
        (Err(error), Ok(_)) | (_, Err(error)) => Err(error),
    }
}

fn join_writer(writer: JoinHandle<io::Result<()>>, allow_broken_pipe: bool) -> TestResult<()> {
    match writer.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) if allow_broken_pipe && error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Ok(Err(error)) => Err(Box::new(error)),
        Err(_) => Err(failure("stdin writer thread panicked")),
    }
}

fn cleanup_after_failure(
    child: &mut Child,
    writer: JoinHandle<io::Result<()>>,
    allow_broken_pipe: bool,
) -> TestResult<()> {
    let cleanup = cleanup_child(child).map_err(|error| Box::new(error) as Box<dyn Error>);
    let write = join_writer(writer, allow_broken_pipe);
    match (cleanup, write) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(cleanup_error), Err(writer_error)) => Err(combined(
            "cleanup and stdin writer failed",
            cleanup_error.as_ref(),
            writer_error.as_ref(),
        )),
    }
}

fn after_cleanup(
    context: &str,
    failure: Box<dyn Error>,
    cleanup: TestResult<()>,
) -> Box<dyn Error> {
    match cleanup {
        Ok(()) => failure,
        Err(cleanup_error) => combined(context, failure.as_ref(), cleanup_error.as_ref()),
    }
}

pub(super) fn invoke(
    arguments: &[&str],
    input: &[u8],
    allow_broken_pipe: bool,
) -> TestResult<Output> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"))
        .args(arguments)
        .env(
            "STS2_WORKER_ENDPOINT_NAMESPACE",
            "preflight-poison-namespace",
        )
        .env("STS2_RUNTIME_PROFILE", "preflight-poison-profile")
        .env("STS2_GATEWAY_ADDR", "preflight-poison-gateway")
        .env("STS2_MCP_BINARY", "/must-not-be-launched")
        .env("STS2_EXO_ADMISSION", "preflight-poison-admission")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let Some(mut stdin) = child.stdin.take() else {
        return match cleanup_child(&mut child) {
            Ok(()) => Err(failure("shipped runtime did not provide piped stdin")),
            Err(error) => Err(Box::new(error)),
        };
    };
    let input = input.to_vec();
    let writer = match thread::Builder::new()
        .name("workflow-fixture-stdin".to_owned())
        .spawn(move || stdin.write_all(&input))
    {
        Ok(writer) => writer,
        Err(spawn_error) => {
            return match cleanup_child(&mut child) {
                Ok(()) => Err(Box::new(spawn_error)),
                Err(cleanup_error) => Err(combined(
                    "stdin writer creation and owned child cleanup failed",
                    &spawn_error,
                    &cleanup_error,
                )),
            };
        }
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let timeout = failure("fixture preflight command exceeded test deadline");
                let cleanup = cleanup_after_failure(&mut child, writer, allow_broken_pipe);
                return Err(after_cleanup(
                    "preflight timeout and cleanup failed",
                    timeout,
                    cleanup,
                ));
            }
            Err(poll_error) => {
                let cleanup = cleanup_after_failure(&mut child, writer, allow_broken_pipe);
                return Err(after_cleanup(
                    "runtime poll and child cleanup failed",
                    Box::new(poll_error),
                    cleanup,
                ));
            }
        }
    }
    let output = child.wait_with_output();
    let writer_result = join_writer(writer, allow_broken_pipe);
    match (output, writer_result) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(Box::new(error)),
        (Ok(_), Err(error)) => Err(error),
        (Err(output_error), Err(writer_error)) => Err(combined(
            "output and stdin writer failed",
            &output_error,
            writer_error.as_ref(),
        )),
    }
}
