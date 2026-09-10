// SPDX-License-Identifier: MIT

fn drain_output<R: Read>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::with_capacity(MAX_CAPTURE_BYTES.min(8192));
    let mut chunk = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if !truncated {
            let remaining = MAX_CAPTURE_BYTES.saturating_sub(output.len());
            if count <= remaining {
                output.extend_from_slice(&chunk[..count]);
            } else {
                output.extend_from_slice(&chunk[..remaining]);
                truncated = true;
            }
        }
    }
    if truncated {
        let keep = MAX_CAPTURE_BYTES.saturating_sub(OUTPUT_TRUNCATION_MARKER.len());
        output.truncate(keep);
        output.extend_from_slice(OUTPUT_TRUNCATION_MARKER);
    }
    Ok(output)
}

fn capture_pipe<R: Read + Send + 'static>(reader: R) -> CaptureHandle {
    thread::spawn(move || drain_output(reader))
}

fn take_capture_pipes(child: &mut Child) -> Result<CapturePipes, Box<dyn std::error::Error>> {
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other(
                "executable composition child stdout was not piped",
            )
            .into());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other(
                "executable composition child stderr was not piped",
            )
            .into());
        }
    };
    Ok((capture_pipe(stdout), capture_pipe(stderr)))
}

fn finish_capture(capture: CaptureHandle) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(capture
        .join()
        .map_err(|_| std::io::Error::other("executable composition output reader panicked"))??)
}

fn stop(mut child: Child) -> Result<Output, Box<dyn std::error::Error>> {
    let (stdout_capture, stderr_capture) = take_capture_pipes(&mut child)?;
    if child.try_wait()?.is_none() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    Ok(Output {
        status,
        stdout: finish_capture(stdout_capture)?,
        stderr: finish_capture(stderr_capture)?,
    })
}

fn bounded(mut command: Command) -> Result<Output, Box<dyn std::error::Error>> {
    command.process_group(0);
    let mut child = command.spawn()?;
    let process_group = child.id();
    let (stdout_capture, stderr_capture) = take_capture_pipes(&mut child)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let group = format!("-{process_group}");
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", group.as_str()])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            let _ = finish_capture(stdout_capture);
            let _ = finish_capture(stderr_capture);
            return Err("runtime deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    Ok(Output {
        status,
        stdout: finish_capture(stdout_capture)?,
        stderr: finish_capture(stderr_capture)?,
    })
}
