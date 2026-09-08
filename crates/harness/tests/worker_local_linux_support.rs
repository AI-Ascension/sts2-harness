// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::{self, Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub(crate) type TestError = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type TestResult<T = ()> = Result<T, TestError>;
pub(crate) const IDENTITY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) struct OwnedChild(Child);

impl OwnedChild {
    pub(crate) fn new(child: Child) -> Self {
        Self(child)
    }

    pub(crate) fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
static TEST_IMAGE_USERS: AtomicUsize = AtomicUsize::new(0);
static TEST_IMAGE_MODE: OnceLock<u32> = OnceLock::new();
static TEST_IMAGE_LOCK: Mutex<()> = Mutex::new(());

/// Cargo test binaries are normally owner-writable. The transport deliberately
/// rejects that release-artifact mode, so tests hold a process-wide guard that
/// makes this already-running image read-only and restores its mode afterward.
struct TestImageGuard {
    path: PathBuf,
}

impl TestImageGuard {
    fn acquire() -> TestResult<Self> {
        let path = std::env::current_exe()?;
        let mode = fs::metadata(&path)?.permissions().mode() & 0o7777;
        let _ = TEST_IMAGE_MODE.set(mode);
        let _lock = TEST_IMAGE_LOCK
            .lock()
            .map_err(|_| std::io::Error::other("test image lock poisoned"))?;
        if TEST_IMAGE_USERS.load(Ordering::SeqCst) == 0 {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode & !0o222))?;
        }
        TEST_IMAGE_USERS.fetch_add(1, Ordering::SeqCst);
        Ok(Self { path })
    }
}

impl Drop for TestImageGuard {
    fn drop(&mut self) {
        let Ok(_lock) = TEST_IMAGE_LOCK.lock() else {
            return;
        };
        if TEST_IMAGE_USERS.fetch_sub(1, Ordering::SeqCst) == 1
            && let Some(mode) = TEST_IMAGE_MODE.get()
        {
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(*mode));
        }
    }
}

pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) endpoint: PathBuf,
    pub(crate) credential: PathBuf,
    pub(crate) secret: Vec<u8>,
    _image_guard: TestImageGuard,
}

impl Fixture {
    pub(crate) fn new(secret: &[u8]) -> TestResult<Self> {
        let image_guard = TestImageGuard::acquire()?;
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
            _image_guard: image_guard,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.endpoint);
        let _ = fs::remove_file(&self.credential);
        let _ = fs::remove_dir(&self.root);
    }
}

#[tokio::test]
async fn support_fixture_and_frame_helpers_roundtrip() -> TestResult {
    let fixture = Fixture::new(b"support-secret")?;
    assert!(IDENTITY_DEADLINE.as_secs() > 0);
    let child = OwnedChild::new(
        Command::new("/bin/sh")
            .args(["-c", "read value"])
            .stdin(Stdio::piped())
            .spawn()?,
    );
    assert!(child.id() > 0);
    let (mut sender, mut receiver) = UnixStream::pair()?;
    let writer = tokio::spawn(async move {
        write_auth(&mut sender, b"support-secret", b"support-auth\0").await?;
        write_frame(&mut sender, b"support-frame").await
    });
    let mut length = [0_u8; 4];
    receiver.read_exact(&mut length).await?;
    let body_length = usize::try_from(u32::from_be_bytes(length))?;
    let mut body = vec![0_u8; body_length];
    receiver.read_exact(&mut body).await?;
    assert_eq!(body, b"support-auth\0support-secret");
    assert_eq!(read_frame(&mut receiver).await?, b"support-frame");
    writer.await??;
    assert_eq!(fixture.secret, b"support-secret");
    Ok(())
}

pub(crate) async fn write_auth(stream: &mut UnixStream, secret: &[u8], magic: &[u8]) -> TestResult {
    let body_length = magic.len().saturating_add(secret.len());
    stream
        .write_all(&(u32::try_from(body_length)?).to_be_bytes())
        .await?;
    stream.write_all(magic).await?;
    stream.write_all(secret).await?;
    Ok(())
}

pub(crate) async fn write_frame(stream: &mut UnixStream, body: &[u8]) -> TestResult {
    stream
        .write_all(&(u32::try_from(body.len())?).to_be_bytes())
        .await?;
    stream.write_all(body).await?;
    Ok(())
}

pub(crate) async fn read_frame(stream: &mut UnixStream) -> TestResult<Vec<u8>> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await?;
    let length = usize::try_from(u32::from_be_bytes(length))?;
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await?;
    Ok(body)
}
