// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]
mod worker_frame_io {
    pub use sts2_harness::worker_frame_io::{ConnectionDeadline, FrameIoError, WorkerFrameIo};
}
mod worker_handoff {
    pub use sts2_harness::worker_handoff::MAX_FRAME_BYTES;
}
#[path = "../src/worker_local_linux.rs"]
mod worker_local_linux;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use sts2_harness::worker_frame_io::ConnectionDeadline;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::sleep;
use worker_handoff::MAX_FRAME_BYTES;
use worker_local_linux::{
    AUTH_MAGIC, LinuxPeerIdentity, LinuxTransportError, LinuxWorkerConfig, MAX_AUTH_BODY_BYTES,
};

type TestError = Box<dyn std::error::Error + Send + Sync>;
type TestResult<T = ()> = Result<T, TestError>;
const IDENTITY_DEADLINE: Duration = Duration::from_secs(5);
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
struct Fixture {
    root: PathBuf,
    endpoint: PathBuf,
    credential: PathBuf,
    secret: Vec<u8>,
}

impl Fixture {
    fn new(secret: &[u8]) -> TestResult<Self> {
        let nonce = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ascension-harness-linux-worker-{}-{nonce}",
            process::id()
        ));
        fs::create_dir(&root)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let credential = root.join("worker-credential");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&credential)?;
        file.write_all(secret)?;
        file.sync_all()?;
        let endpoint = root.join("worker.sock");
        Ok(Self {
            root,
            endpoint,
            credential,
            secret: secret.to_vec(),
        })
    }

    fn config(&self) -> TestResult<LinuxWorkerConfig> {
        Ok(LinuxWorkerConfig::new(
            self.endpoint.clone(),
            self.credential.clone(),
            peer_variant(4)?,
        )?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.endpoint);
        let _ = fs::remove_file(&self.credential);
        let _ = fs::remove_dir(&self.root);
    }
}

fn peer_variant(mutation: u8) -> TestResult<LinuxPeerIdentity> {
    let mut uid = rustix::process::getuid().as_raw();
    let mut pid = process::id();
    let mut start_token = read_start_token(pid)?;
    let executable = fs::read_link(format!("/proc/{pid}/exe"))?;
    let mut digest = image_digest()?;
    match mutation {
        0 => uid = peer_uid_plus_one(),
        1 => pid = pid.saturating_add(1),
        2 => start_token = start_token.saturating_add(1),
        3 => digest[0] ^= 1,
        _ => {}
    }
    Ok(LinuxPeerIdentity::new(
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

async fn write_auth(stream: &mut UnixStream, secret: &[u8]) -> TestResult {
    let body_length = AUTH_MAGIC.len().saturating_add(secret.len());
    stream
        .write_all(&(u32::try_from(body_length)?).to_be_bytes())
        .await?;
    stream.write_all(AUTH_MAGIC).await?;
    stream.write_all(secret).await?;
    Ok(())
}

async fn write_frame(stream: &mut UnixStream, body: &[u8]) -> TestResult {
    stream
        .write_all(&(u32::try_from(body.len())?).to_be_bytes())
        .await?;
    stream.write_all(body).await?;
    Ok(())
}

async fn read_frame(stream: &mut UnixStream) -> TestResult<Vec<u8>> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await?;
    let length = usize::try_from(u32::from_be_bytes(length))?;
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

#[tokio::test]
async fn positive_process_peer_auth_and_one_frame_roundtrip() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture.config()?.bind()?;
    assert_eq!(
        fs::metadata(&fixture.endpoint)?.permissions().mode() & 0o777,
        0o600
    );
    let deadline = ConnectionDeadline::start(IDENTITY_DEADLINE)?;
    let endpoint = fixture.endpoint.clone();
    let secret = fixture.secret.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(endpoint).await?;
        write_auth(&mut stream, &secret).await?;
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
    let listener = fixture.config()?.bind()?;
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
    let listener = fixture.config()?.bind()?;
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
        let listener = fixture.config()?.bind()?;
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
                let _ = write_auth(&mut stream, &secret).await;
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
    let listener = fixture.config()?.bind()?;
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
    write_auth(&mut stream, &fixture.secret).await?;
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
    let listener = fixture.config()?.bind()?;
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
        fixture.config()?.bind(),
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
async fn oversized_request_poisoning_cannot_be_restarted() -> TestResult {
    let fixture = Fixture::new(b"worker-control-secret")?;
    let listener = fixture.config()?.bind()?;
    let endpoint = fixture.endpoint.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(endpoint).await?;
        write_auth(&mut stream, b"worker-control-secret").await?;
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

fn image_digest() -> TestResult<[u8; 32]> {
    let executable = fs::read_link(format!("/proc/{}/exe", process::id()))?;
    let mut file = File::open(executable)?;
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

fn peer_uid_plus_one() -> u32 {
    rustix::process::getuid().as_raw().saturating_add(1)
}
