// SPDX-License-Identifier: MIT

use super::{
    AUTH_MAGIC, ConnectionDeadline, Duration, Fixture, IDENTITY_DEADLINE, LinuxTransportError,
    TestResult, UnixStream, fixture_config, sleep, write_auth,
};

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
