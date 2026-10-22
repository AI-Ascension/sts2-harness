// SPDX-License-Identifier: MIT

use super::*;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

struct Probe {
    root: PathBuf,
    executable: PathBuf,
}

impl Probe {
    fn new(script: &[u8]) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let parent = PathBuf::from("/tmp/ai-ascension-exo-package-147-tests");
        match fs::DirBuilder::new().mode(0o700).create(&parent) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        super::fs::open_directory(&parent, true)?;
        let root = parent.join(uuid::Uuid::new_v4().simple().to_string());
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let executable = root.join("probe");
        fs::write(&executable, script)?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
        Ok(Self { root, executable })
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn probe_preserves_bounded_stdout_and_never_consumes_input() -> TestResult {
    let probe =
        Probe::new(b"#!/bin/sh\nif read -r input; then exit 91; fi\nprintf 'v22.14.0\\n'\n")?;
    assert_eq!(
        invoke(&probe.executable, &["--version"], &probe.root, 64)?,
        b"v22.14.0\n"
    );
    Ok(())
}

#[test]
fn failed_and_oversized_probe_outputs_remain_redacted() -> TestResult {
    let failed = Probe::new(b"#!/bin/sh\nprintf private-marker >&2\nexit 7\n")?;
    assert_eq!(
        invoke(&failed.executable, &[], &failed.root, 64),
        Err("exo_package_probe_failed")
    );
    let oversized = Probe::new(b"#!/bin/sh\nprintf 01234567890123456789\n")?;
    assert_eq!(
        invoke(&oversized.executable, &[], &oversized.root, 4),
        Err("exo_package_probe_output_bound")
    );
    Ok(())
}

#[test]
fn stalled_owned_probe_is_killed_and_reaped_at_the_deadline() -> TestResult {
    // The shell is the direct child and creates no descendant, including on timeout.
    let probe = Probe::new(b"#!/bin/sh\nwhile :; do :; done\n")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    assert_eq!(
        runtime.block_on(invoke_bounded(
            &probe.executable,
            &[],
            &probe.root,
            64,
            Duration::from_millis(25)
        )),
        Err("exo_package_probe_timeout")
    );
    Ok(())
}
