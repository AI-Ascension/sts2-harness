// SPDX-License-Identifier: MIT

//! Process-level regression coverage for the verifier's exact self-reexec.

use super::spawn_fixed_verifier;
use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, copy};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MEMFD_CONTROLLER_ENTRY_ENV: &str = "ASCENSION_TEST_VERIFIER_MEMFD_CONTROLLER_ENTRY";
const MEMFD_CONTROLLER_ENTRY_NAME: &str =
    "worker_local_linux::worker_linux_verifier::spawn::tests::sealed_memfd_controller_entry";
const MAX_TEST_IMAGE_BYTES: u64 = 128 * 1024 * 1024;
const CHILD_REAP_BUDGET: Duration = Duration::from_secs(1);
const CHILD_WAIT_BUDGET: Duration = Duration::from_secs(5);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(5);

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if self.0.kill().is_err() {
                    eprintln!("verifier regression child cleanup is uncertain");
                    return;
                }
                let deadline = Instant::now() + CHILD_REAP_BUDGET;
                loop {
                    match self.0.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(CHILD_POLL_INTERVAL);
                        }
                        Ok(None) => {
                            eprintln!("verifier regression child remained live after cleanup");
                            break;
                        }
                        Err(_) => {
                            eprintln!("verifier regression child cleanup became uncertain");
                            break;
                        }
                    }
                }
            }
            Err(_) => eprintln!("verifier regression child status is uncertain"),
        }
    }
}

impl ChildGuard {
    fn wait_bounded(
        &mut self,
        deadline: Instant,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        loop {
            match self.0.try_wait()? {
                Some(status) => return Ok(Some(status)),
                None if Instant::now() < deadline => std::thread::sleep(CHILD_POLL_INTERVAL),
                None => return Ok(None),
            }
        }
    }
}

/// This entry is launched from a sealed memfd by the parent test below. It
/// then exercises the production controller launch seam, so a deleted memfd
/// path would fail before the verifier helper can close its private channel.
#[test]
fn sealed_memfd_controller_entry() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os(MEMFD_CONTROLLER_ENTRY_ENV).is_none() {
        return Ok(());
    }
    let executable = std::fs::read_link("/proc/self/exe")?;
    assert!(
        executable.to_string_lossy().starts_with("/memfd:"),
        "fixture must run from a sealed executable memfd"
    );

    let (owner, child_control) = rustix::net::socketpair(
        rustix::net::AddressFamily::UNIX,
        rustix::net::SocketType::SEQPACKET,
        rustix::net::SocketFlags::CLOEXEC | rustix::net::SocketFlags::NONBLOCK,
        None,
    )?;
    let helper = spawn_fixed_verifier(child_control)
        .map_err(|_| std::io::Error::other("verifier helper spawn failed"))?;
    let mut helper = ChildGuard(helper);
    drop(owner);
    let status = helper
        .wait_bounded(Instant::now() + CHILD_WAIT_BUDGET)?
        .ok_or_else(|| std::io::Error::other("verifier helper exceeded its wait budget"))?;
    assert!(status.success(), "verifier helper did not exit cleanly");
    Ok(())
}

#[test]
fn sealed_memfd_parent_can_start_verifier_helper() -> Result<(), Box<dyn std::error::Error>> {
    let source = std::env::current_exe()?;
    let mut image = executable_memfd()?;
    let source = File::open(&source)?;
    let metadata = source.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_TEST_IMAGE_BYTES {
        return Err(std::io::Error::other("test executable exceeds image bound").into());
    }
    let mut bounded_source = source.take(MAX_TEST_IMAGE_BYTES + 1);
    let copied = copy(&mut bounded_source, &mut image)?;
    if copied > MAX_TEST_IMAGE_BYTES {
        return Err(std::io::Error::other("test executable exceeded image bound").into());
    }
    image.seek(SeekFrom::Start(0))?;
    fcntl_add_seals(
        &image,
        SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
    )?;
    let mut child = ChildGuard(
        Command::new(format!("/proc/self/fd/{}", image.as_raw_fd()))
            .env_clear()
            .args(["--exact", MEMFD_CONTROLLER_ENTRY_NAME, "--nocapture"])
            .env(MEMFD_CONTROLLER_ENTRY_ENV, "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = child.wait_bounded(deadline)?.ok_or_else(|| {
        std::io::Error::other("sealed memfd controller entry exceeded its deadline")
    })?;
    if !status.success() {
        return Err(std::io::Error::other("sealed memfd controller entry failed").into());
    }
    Ok(())
}

fn executable_memfd() -> Result<File, Box<dyn std::error::Error>> {
    let base = MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING;
    let descriptor = match memfd_create("ascension-verifier-test", base | MemfdFlags::EXEC) {
        Ok(descriptor) => descriptor,
        Err(error) if error == rustix::io::Errno::INVAL => {
            memfd_create("ascension-verifier-test", base)?
        }
        Err(error) => return Err(error.into()),
    };
    Ok(File::from(descriptor))
}
