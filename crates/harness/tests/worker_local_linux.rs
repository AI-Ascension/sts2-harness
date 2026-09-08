// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]
mod worker_frame_io {
    pub use sts2_harness::worker_frame_io::{ConnectionDeadline, FrameIoError, WorkerFrameIo};
}
mod worker_handoff {
    pub use sts2_harness::worker_handoff::MAX_FRAME_BYTES;
}
#[path = "worker_local_linux_support.rs"]
mod support;
#[path = "../src/worker_local_linux.rs"]
mod worker_local_linux;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::Path;
use std::process;
use std::time::Duration;
use sts2_harness::worker_frame_io::ConnectionDeadline;
use support::{
    Fixture, IDENTITY_DEADLINE, TestError, TestResult, read_frame, write_auth, write_frame,
};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::time::sleep;
use worker_handoff::MAX_FRAME_BYTES;
use worker_local_linux::{AUTH_MAGIC, LinuxTransportError, LinuxWorkerConfig, MAX_AUTH_BODY_BYTES};

fn peer_variant(mutation: u8) -> TestResult<worker_local_linux::LinuxPeerIdentity> {
    let mut uid = rustix::process::getuid().as_raw();
    let mut pid = process::id();
    let mut start_token = read_start_token(pid)?;
    let executable = fs::read_link(format!("/proc/{pid}/exe"))?;
    let mut digest = image_digest(&executable)?;
    match mutation {
        0 => uid = rustix::process::getuid().as_raw().saturating_add(1),
        1 => pid = pid.saturating_add(1),
        2 => start_token = start_token.saturating_add(1),
        3 => digest[0] ^= 1,
        _ => {}
    }
    Ok(worker_local_linux::LinuxPeerIdentity::new(
        uid,
        pid,
        start_token,
        executable,
        digest,
    )?)
}

fn read_start_token(pid: u32) -> TestResult<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let close = text.rfind(')').ok_or("missing process comm terminator")?;
    let token = text
        .get(close + 2..)
        .and_then(|suffix| suffix.split_whitespace().nth(19))
        .ok_or("missing process start token")?
        .parse::<u64>()?;
    Ok(token)
}

fn image_digest(path: &Path) -> TestResult<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hasher.update(&bytes[..count]);
    }
    Ok(hasher.finalize().into())
}

fn fixture_config(fixture: &Fixture) -> TestResult<LinuxWorkerConfig> {
    Ok(LinuxWorkerConfig::new(
        fixture.endpoint.clone(),
        fixture.credential.clone(),
        peer_variant(4)?,
    )?)
}

#[tokio::test]
async fn positive_process_peer_auth_and_one_frame_roundtrip() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    assert_eq!(
        fs::metadata(&fixture.endpoint)?.permissions().mode() & 0o777,
        0o600
    );
    let deadline = ConnectionDeadline::start(IDENTITY_DEADLINE)?;
    let endpoint = fixture.endpoint.clone();
    let secret = fixture.secret.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(endpoint).await?;
        write_auth(&mut stream, &secret, AUTH_MAGIC).await?;
        write_frame(&mut stream, b"{\"scope\":\"probe\"}").await?;
        read_frame(&mut stream).await
    });
    let mut connection = listener.accept_authenticated(deadline).await?;
    let _witness = connection.peer_witness();
    assert_eq!(
        connection.read_request_bytes().await?,
        b"{\"scope\":\"probe\"}"
    );
    connection
        .write_response_bytes(b"{\"ready\":false}")
        .await?;
    assert_eq!(client.await??, b"{\"ready\":false}");
    Ok(())
}

#[tokio::test]
async fn concurrent_accept_is_rejected_busy() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    let deadline = ConnectionDeadline::start(IDENTITY_DEADLINE)?;
    let mut pending = Box::pin(listener.accept_authenticated(deadline));
    let completed = tokio::select! {
        result = &mut pending => Some(result),
        _ = sleep(Duration::from_millis(20)) => None,
    };
    assert!(completed.is_none());
    assert!(matches!(
        listener
            .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
            .await,
        Err(LinuxTransportError::Busy)
    ));
    drop(pending);
    Ok(())
}

#[tokio::test]
async fn wrong_uid_pid_start_and_image_fail_before_credential_read() -> TestResult {
    for mutation in 0_u8..4 {
        let fixture = Fixture::new(b"worker-control-secret")?;
        let peer = peer_variant(mutation)?;
        let config =
            LinuxWorkerConfig::new(fixture.endpoint.clone(), fixture.credential.clone(), peer)?;
        if mutation == 3 {
            assert!(matches!(
                config.bind(),
                Err(LinuxTransportError::Configuration)
            ));
            continue;
        }
        let listener = config.bind()?;
        let endpoint = fixture.endpoint.clone();
        let client = tokio::spawn(async move {
            let _ = UnixStream::connect(endpoint).await;
            Ok::<(), TestError>(())
        });
        let error = listener
            .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
            .await
            .err()
            .ok_or("wrong peer identity unexpectedly authenticated")?;
        assert_eq!(error, LinuxTransportError::Peer);
        client.await??;
    }
    Ok(())
}

#[tokio::test]
async fn dead_peer_and_bad_credentials_receive_no_request() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    let endpoint = fixture.endpoint.clone();
    let client = tokio::spawn(async move {
        let stream = UnixStream::connect(endpoint).await?;
        drop(stream);
        Ok::<(), TestError>(())
    });
    let error = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await
        .err()
        .ok_or("dead peer unexpectedly authenticated")?;
    assert_eq!(error, LinuxTransportError::Closed);
    client.await??;

    let endpoint = fixture.endpoint.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(endpoint).await?;
        let body_length = AUTH_MAGIC.len() + 3;
        stream
            .write_all(&(u32::try_from(body_length)?).to_be_bytes())
            .await?;
        stream.write_all(AUTH_MAGIC).await?;
        stream.write_all(b"bad").await?;
        Ok::<(), TestError>(())
    });
    let error = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await
        .err()
        .ok_or("bad credential unexpectedly authenticated")?;
    assert_eq!(error, LinuxTransportError::Credential);
    client.await??;
    Ok(())
}

#[tokio::test]
async fn malformed_oversized_and_trickled_preludes_are_bounded() -> TestResult {
    let cases = [
        (0_u32, None),
        (u32::try_from(MAX_AUTH_BODY_BYTES + 1)?, None),
        (u32::try_from(AUTH_MAGIC.len() + 1)?, Some(1_u8)),
    ];
    for (length, first_byte) in cases {
        let fixture = Fixture::new(b"worker-control-secret")?;
        let listener = fixture_config(&fixture)?.bind()?;
        let endpoint = fixture.endpoint.clone();
        let secret = fixture.secret.clone();
        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(endpoint).await?;
            stream.write_all(&length.to_be_bytes()).await?;
            if let Some(byte) = first_byte {
                stream.write_all(&[byte]).await?;
                sleep(Duration::from_secs(6)).await;
            }
            if length == u32::try_from(AUTH_MAGIC.len() + 1)? {
                let _ = write_auth(&mut stream, &secret, AUTH_MAGIC).await;
            }
            Ok::<(), TestError>(())
        });
        let error = listener
            .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
            .await
            .err()
            .ok_or("malformed/trickled prelude unexpectedly authenticated")?;
        assert!(matches!(
            error,
            LinuxTransportError::Credential | LinuxTransportError::Deadline
        ));
        let _ = client.await;
    }
    Ok(())
}

#[tokio::test]
async fn cancellation_drops_auth_stream_and_keeps_listener_usable() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    let endpoint = fixture.endpoint.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(endpoint).await?;
        stream.write_all(&[0, 0, 0, 26]).await?;
        stream.write_all(&[AUTH_MAGIC[0]]).await?;
        sleep(Duration::from_millis(100)).await;
        Ok::<(), TestError>(())
    });
    let deadline = ConnectionDeadline::start(IDENTITY_DEADLINE)?;
    let mut pending = Box::pin(listener.accept_authenticated(deadline));
    let cancelled = tokio::select! {
        result = &mut pending => Some(result),
        _ = sleep(Duration::from_millis(20)) => None,
    };
    assert!(cancelled.is_none());
    drop(pending);
    client.await??;

    let mut stream = UnixStream::connect(&fixture.endpoint).await?;
    write_auth(&mut stream, &fixture.secret, AUTH_MAGIC).await?;
    write_frame(&mut stream, b"{}\n").await?;
    let mut connection = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await?;
    assert_eq!(connection.read_request_bytes().await?, b"{}\n");
    connection.write_response_bytes(b"ok").await?;
    assert_eq!(read_frame(&mut stream).await?, b"ok");
    Ok(())
}

#[tokio::test]
async fn endpoint_path_replacement_is_rejected_before_accept() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    fs::remove_file(&fixture.endpoint)?;
    let replacement = StdUnixListener::bind(&fixture.endpoint)?;
    replacement.set_nonblocking(true)?;
    let error = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await
        .err()
        .ok_or("replaced endpoint unexpectedly accepted")?;
    assert_eq!(error, LinuxTransportError::Configuration);
    drop(replacement);
    Ok(())
}

#[test]
fn protected_path_and_credential_permissions_fail_closed() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    fs::set_permissions(&fixture.credential, fs::Permissions::from_mode(0o644))?;
    assert!(matches!(
        fixture_config(&fixture)?.bind(),
        Err(LinuxTransportError::Credential)
    ));
    fs::set_permissions(&fixture.credential, fs::Permissions::from_mode(0o600))?;

    let linked_root = fixture.root.join("linked");
    let real_root = fixture.root.join("real");
    fs::create_dir(&real_root)?;
    fs::set_permissions(&real_root, fs::Permissions::from_mode(0o700))?;
    std::os::unix::fs::symlink(&real_root, &linked_root)?;
    let endpoint = linked_root.join("worker.sock");
    let config = LinuxWorkerConfig::new(endpoint, fixture.credential.clone(), peer_variant(4)?)?;
    assert!(matches!(
        config.bind(),
        Err(LinuxTransportError::Configuration)
    ));
    Ok(())
}

#[tokio::test]
async fn cancelled_request_read_closes_exchange() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    let mut stream = UnixStream::connect(&fixture.endpoint).await?;
    write_auth(&mut stream, &fixture.secret, AUTH_MAGIC).await?;
    let mut connection = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await?;
    let mut pending = Box::pin(connection.read_request_bytes());
    tokio::select! {
        result = &mut pending => return Err(format!("read completed before cancellation: {result:?}").into()),
        _ = sleep(Duration::from_millis(20)) => {}
    }
    drop(pending);
    assert_eq!(
        connection.read_request_bytes().await,
        Err(LinuxTransportError::Closed)
    );
    assert_eq!(
        connection.write_response_bytes(b"x").await,
        Err(LinuxTransportError::Closed)
    );
    Ok(())
}

#[tokio::test]
async fn oversized_request_poisoning_cannot_be_restarted() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture_config(&fixture)?.bind()?;
    let endpoint = fixture.endpoint.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(endpoint).await?;
        write_auth(&mut stream, b"worker-control-secret", AUTH_MAGIC).await?;
        stream
            .write_all(&u32::try_from(MAX_FRAME_BYTES + 1)?.to_be_bytes())
            .await?;
        Ok::<(), TestError>(())
    });
    let mut connection = listener
        .accept_authenticated(ConnectionDeadline::start(IDENTITY_DEADLINE)?)
        .await?;
    assert_eq!(
        connection.read_request_bytes().await,
        Err(LinuxTransportError::Framing)
    );
    assert_eq!(
        connection.write_response_bytes(b"x").await,
        Err(LinuxTransportError::Closed)
    );
    client.await??;
    Ok(())
}
