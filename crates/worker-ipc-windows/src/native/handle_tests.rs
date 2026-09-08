// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::FALSE;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

use super::test_support::Fixture;
use crate::{EndpointPolicy, ExpectedPeer, Sid, TransportError, WorkerListener};

fn handle_count() -> Result<u32, TransportError> {
    let mut count = 0;
    // The current-process pseudo handle is borrowed, not closed. The output
    // pointer refers to an aligned live u32 for this synchronous native call.
    let result = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
    if result == FALSE {
        Err(TransportError::Os)
    } else {
        Ok(count)
    }
}

fn reject_bad_image(fixture: &Fixture) -> Result<(), TransportError> {
    let nonce = super::test_support::nonce(64);
    let actual = fixture.policy_for_current(nonce)?;
    let expected = actual.expected_peer();
    let peer = ExpectedPeer::new(
        Sid::new(fixture.worker_sid.clone())?,
        expected.pid(),
        expected.creation_filetime(),
        fixture.image.clone(),
        [0_u8; 32],
        nonce,
    )?;
    let policy = EndpointPolicy::new(
        Sid::new(fixture.worker_sid.clone())?,
        peer,
        fixture.credential.clone(),
    )?;
    assert!(matches!(
        WorkerListener::bind(policy),
        Err(TransportError::Identity)
    ));
    Ok(())
}

#[test]
fn native_failed_image_preparation_releases_handles() -> Result<(), TransportError> {
    // Count in an isolated test process so unrelated parallel tests cannot
    // open or close handles during this oracle's measurement interval.
    if std::env::var_os("WORKER_IPC_HANDLE_COUNT_CHILD").is_none() {
        let status =
            std::process::Command::new(std::env::current_exe().map_err(|_| TransportError::Os)?)
                .args([
                    "--exact",
                    "native::handle_tests::native_failed_image_preparation_releases_handles",
                ])
                .env("WORKER_IPC_HANDLE_COUNT_CHILD", "1")
                .status()
                .map_err(|_| TransportError::Os)?;
        assert!(status.success());
        return Ok(());
    }
    let fixture = Fixture::new()?;
    reject_bad_image(&fixture)?;
    let baseline = handle_count()?;
    for _ in 0..16 {
        reject_bad_image(&fixture)?;
        assert_eq!(handle_count()?, baseline);
    }
    Ok(())
}
