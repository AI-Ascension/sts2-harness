// SPDX-License-Identifier: MIT

use super::{
    AUTH_MAGIC, ConnectionDeadline, Duration, Fixture, IDENTITY_DEADLINE, LinuxTransportError,
    TestResult, UnixStream, fixture_config, sleep, write_auth,
};

struct OwnedExecWriter(std::process::Child);

impl Drop for OwnedExecWriter {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn exec_writer_entry() -> TestResult {
    use std::io::{Read, Write};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let Some(endpoint) = std::env::var_os("ASC_TEST_EXEC_ENDPOINT") else {
        return Ok(());
    };
    let mut signal = [0_u8; 1];
    std::io::stdin().read_exact(&mut signal)?;
    if signal != *b"C" {
        return Err("unexpected connect signal".into());
    }
    let mut stream = std::os::unix::net::UnixStream::connect(endpoint)?;
    let mut auth = AUTH_MAGIC.to_vec();
    auth.extend_from_slice(b"worker-control-secret");
    stream.write_all(&u32::try_from(auth.len())?.to_be_bytes())?;
    stream.write_all(&auth)?;
    std::io::stdin().read_exact(&mut signal)?;
    if signal != *b"E" {
        return Err("unexpected exec signal".into());
    }
    let socket = std::os::fd::OwnedFd::from(stream);
    let error = Command::new("/usr/bin/cat")
        .env_clear()
        .stdin(Stdio::inherit())
        .stdout(Stdio::from(socket))
        .stderr(Stdio::null())
        .exec();
    Err(error.into())
}

#[tokio::test]
async fn same_pid_exec_after_authentication_rejects_request() -> TestResult {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::process::{Command, Stdio};
    let fixture = Fixture::new(b"worker-control-secret")?;
    let executable = std::env::current_exe()?;
    let digest = Sha256::digest(std::fs::read(&executable)?).into();
    let mut writer = OwnedExecWriter(
        Command::new(&executable)
            .args(["--exact", "inheritance::exec_writer_entry", "--nocapture"])
            .env_clear()
            .env("ASC_TEST_EXEC_ENDPOINT", &fixture.endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let pid = writer.0.id();
    let peer = super::worker_local_linux::LinuxPeerIdentity::new(
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw(),
        pid,
        super::read_start_token(pid)?,
        executable,
        digest,
    )?;
    let listener =
        super::LinuxWorkerConfig::new(fixture.endpoint.clone(), fixture.credential.clone(), peer)?
            .bind()?;
    let input = writer
        .0
        .stdin
        .as_mut()
        .ok_or("missing child control pipe")?;
    input.write_all(b"C")?;
    let mut connection = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await?;
    input.write_all(b"E")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if std::fs::read_link(format!("/proc/{pid}/exe"))? == std::path::Path::new("/usr/bin/cat") {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("synthetic peer did not exec".into());
        }
        sleep(Duration::from_millis(5)).await;
    }
    // The replacement remains alive, with the same PID/UID/GID and connected
    // socket. Only the approved executable identity is now wrong.
    input.write_all(b"\0\0\0\x02{}")?;
    assert_eq!(
        connection.read_request_bytes().await,
        Err(LinuxTransportError::Peer)
    );
    assert_eq!(
        connection.write_response_bytes(b"x").await,
        Err(LinuxTransportError::Closed)
    );
    assert!(writer.0.try_wait()?.is_none());
    Ok(())
}

#[tokio::test]
async fn inherited_connected_descriptor_cannot_send_a_worker_request() -> TestResult {
    use std::os::fd::AsFd;
    use std::process::{Child, Command, Stdio};
    struct OwnedWriter(Child);
    impl Drop for OwnedWriter {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    let mut client = UnixStream::connect(&fixture.endpoint).await?;
    write_auth(&mut client, &fixture.secret, AUTH_MAGIC).await?;
    let mut connection = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await?;
    // The approved parent remains alive and retains its connection. Only the
    // writer changes: this child emits one otherwise-valid length-prefixed JSON
    // body through the same connected socket after authentication has completed.
    let socket = std::fs::File::from(client.as_fd().try_clone_to_owned()?);
    let mut writer = OwnedWriter(
        Command::new("/usr/bin/printf")
            .env_clear()
            .arg("\\000\\000\\000\\002{}")
            .stdin(Stdio::null())
            .stdout(Stdio::from(socket))
            .stderr(Stdio::null())
            .spawn()?,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = writer.0.try_wait()? {
            assert!(status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "synthetic writer did not exit"
        );
        sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        connection.read_request_bytes().await,
        Err(LinuxTransportError::Io)
    );
    assert_eq!(
        connection.write_response_bytes(b"x").await,
        Err(LinuxTransportError::Closed)
    );
    Ok(())
}
