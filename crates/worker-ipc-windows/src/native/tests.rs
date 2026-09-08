// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

//! Native-only transport tests.
//!
//! The support module uses the current test executable as a finite peer.  The
//! peer is never given credential bytes through arguments or environment; it
//! reads the same protected fixture file that the listener validates.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::FALSE;
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use super::io;
use super::process::{process_creation, process_sid, query_process_image};
use super::resources::{Handle, HeldImage, ProtectedCredential};
use super::security::PipeSecurity;
use super::test_peer::{peer_exchange, spawn_peer};
use super::test_support::{Fixture, pipe_name};
use crate::{AuthenticatedConnection, Deadline, TransportError, WorkerListener};

#[test]
fn peer_client() {
    let (Some(name), Some(path), Some(mode)) = (
        std::env::var("WORKER_IPC_TEST_PIPE_NAME").ok(),
        std::env::var("WORKER_IPC_TEST_CREDENTIAL_PATH").ok(),
        std::env::var("WORKER_IPC_TEST_PEER_MODE").ok(),
    ) else {
        return;
    };
    assert!(peer_exchange(&mode, &name, Path::new(&path)));
}

#[test]
fn native_finite_peer_completes_one_exchange() -> Result<(), TransportError> {
    let fixture = Fixture::new()?;
    let endpoint_nonce = super::test_support::nonce(1);
    let mut child = spawn_peer("valid", endpoint_nonce, &fixture.credential)?;
    let policy = fixture.policy_for_child(&child, endpoint_nonce)?;
    assert_eq!(policy.pipe_name(), pipe_name(endpoint_nonce));
    let mut listener = WorkerListener::bind(policy)?;
    let mut connection = listener.accept_authenticated(Deadline::new(Duration::from_secs(2))?)?;
    assert_eq!(connection.read_request()?, b"{}");
    connection.write_response(b"{}")?;
    let status = child.wait().map_err(|_| TransportError::Os)?;
    assert!(status.success());
    listener.shutdown()
}

#[test]
fn native_malformed_auth_and_deadline_are_bounded() -> Result<(), TransportError> {
    for (index, mode) in ["wrong-magic", "oversized-auth", "slow-auth"]
        .into_iter()
        .enumerate()
    {
        let fixture = Fixture::new()?;
        let endpoint_nonce = super::test_support::nonce(10 + index as u64);
        let mut child = spawn_peer(mode, endpoint_nonce, &fixture.credential)?;
        let policy = fixture.policy_for_child(&child, endpoint_nonce)?;
        let mut listener = WorkerListener::bind(policy)?;
        let result = listener.accept_authenticated(Deadline::new(if mode == "slow-auth" {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(2)
        })?);
        match mode {
            "wrong-magic" => assert!(matches!(result, Err(TransportError::Credential))),
            "oversized-auth" => assert!(matches!(result, Err(TransportError::Framing))),
            "slow-auth" => assert!(matches!(result, Err(TransportError::Deadline))),
            _ => return Err(TransportError::Configuration),
        }
        let _ = child.wait();
        listener.shutdown()?;
    }
    Ok(())
}

#[test]
fn native_peer_pid_creation_and_sid_mismatches_fail_closed() -> Result<(), TransportError> {
    for (index, mismatch) in ["pid", "creation", "sid"].into_iter().enumerate() {
        let fixture = Fixture::new()?;
        let endpoint_nonce = super::test_support::nonce(15 + index as u64);
        let mut child = spawn_peer("valid", endpoint_nonce, &fixture.credential)?;
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, child.id()) };
        let process = Handle::new(process).map_err(|_| TransportError::Os)?;
        let creation = process_creation(process.raw())?;
        let actual_sid = super::test_support::sid_text_for_test(&process_sid(process.raw())?)?;
        let image = query_process_image(process.raw())?;
        let pid = if mismatch == "pid" {
            child.id().saturating_add(1)
        } else {
            child.id()
        };
        let expected_creation = if mismatch == "creation" {
            creation.saturating_add(1)
        } else {
            creation
        };
        let expected_sid = if mismatch == "sid" {
            if actual_sid == "S-1-5-18" {
                "S-1-5-32-544".to_owned()
            } else {
                "S-1-5-18".to_owned()
            }
        } else {
            actual_sid
        };
        let policy = fixture.policy_for_child_parts(
            pid,
            endpoint_nonce,
            expected_sid,
            expected_creation,
            image,
            fixture.image_sha256,
        )?;
        let mut listener = WorkerListener::bind(policy)?;
        let result = listener.accept_authenticated(Deadline::new(Duration::from_millis(500))?);
        assert!(matches!(result, Err(TransportError::Identity)));
        let _ = child.wait();
        listener.shutdown()?;
    }
    let fixture = Fixture::new()?;
    assert!(matches!(
        HeldImage::open(&fixture.image, &[0_u8; 32]),
        Err(TransportError::Identity)
    ));
    Ok(())
}

#[test]
fn native_shutdown_cancels_and_joins_active_reader() -> Result<(), TransportError> {
    let fixture = Fixture::new()?;
    let endpoint_nonce = super::test_support::nonce(20);
    let mut child = spawn_peer("slow-frame", endpoint_nonce, &fixture.credential)?;
    let policy = fixture.policy_for_child(&child, endpoint_nonce)?;
    let listener = WorkerListener::bind(policy)?;
    let (accepted_tx, accepted_rx) = mpsc::sync_channel(1);
    let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(1);
    let (shutdown_result_tx, shutdown_result_rx) = mpsc::sync_channel(1);
    let accept_thread = thread::spawn(move || {
        let mut listener = listener;
        let accepted = Deadline::new(Duration::from_secs(2))
            .and_then(|deadline| listener.accept_authenticated(deadline));
        let _ = accepted_tx.send(accepted);
        let _ = shutdown_rx.recv();
        let result = listener.shutdown();
        let _ = shutdown_result_tx.send(result);
    });
    let mut connection: AuthenticatedConnection = accepted_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| TransportError::Os)??;
    let read_thread = thread::spawn(move || connection.read_request());
    thread::sleep(Duration::from_millis(20));
    shutdown_tx.send(()).map_err(|_| TransportError::Os)?;
    let shutdown_result = shutdown_result_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| TransportError::Os)?;
    assert!(shutdown_result.is_ok());
    let read_result = read_thread.join().map_err(|_| TransportError::Os)?;
    assert!(matches!(
        read_result,
        Err(TransportError::Closed | TransportError::Deadline | TransportError::Os)
    ));
    accept_thread.join().map_err(|_| TransportError::Os)?;
    let _ = child.wait();
    Ok(())
}

#[test]
fn native_first_instance_acl_and_reparse_guards_hold() -> Result<(), TransportError> {
    let fixture = Fixture::new()?;
    let first_nonce = super::test_support::nonce(30);
    let first_policy = fixture.policy_for_current(first_nonce)?;
    let second_policy = fixture.policy_for_current(first_nonce)?;
    let mut listener = WorkerListener::bind(first_policy)?;
    assert!(matches!(
        WorkerListener::bind(second_policy),
        Err(TransportError::Os)
    ));
    listener.shutdown()?;

    let hardlink = fixture.directory.join("credential-hardlink.txt");
    fs::hard_link(&fixture.credential, &hardlink).map_err(|_| TransportError::Os)?;
    assert!(matches!(
        ProtectedCredential::open(&hardlink, &fixture.worker_sid),
        Err(TransportError::Credential)
    ));

    let broad = fixture.directory.join("broad.txt");
    fs::write(&broad, b"broad-secret").map_err(|_| TransportError::Os)?;
    assert!(matches!(
        ProtectedCredential::open(&broad, &fixture.worker_sid),
        Err(TransportError::Credential)
    ));

    let reparse = fixture.directory.join("credential-link.txt");
    if std::os::windows::fs::symlink_file(&fixture.credential, &reparse).is_ok() {
        assert!(matches!(
            ProtectedCredential::open(&reparse, &fixture.worker_sid),
            Err(TransportError::Credential)
        ));
    }
    Ok(())
}

#[test]
fn native_unauthorized_client_is_denied_by_pipe_acl() -> Result<(), TransportError> {
    let fixture = Fixture::new()?;
    if fixture.worker_sid == "S-1-5-18" {
        return Ok(());
    }
    let endpoint_nonce = super::test_support::nonce(40);
    let name = pipe_name(endpoint_nonce);
    let security = PipeSecurity::new("S-1-5-18", "S-1-5-18")?;
    let pipe = io::create_pipe(&name, &security, true)?;
    let mut child = spawn_peer("connect-only", endpoint_nonce, &fixture.credential)?;
    let status = child.wait().map_err(|_| TransportError::Os)?;
    drop(pipe);
    assert!(status.success());
    Ok(())
}
