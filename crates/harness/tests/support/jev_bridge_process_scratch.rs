// SPDX-License-Identifier: MIT

//! The private per-case directory the System One bridge process cases run against.
//!
//! Each case gets its own root so a transport, the request it captured, and the marker it writes
//! cannot collide across cases. The marker is what lets a case assert its transport actually ran
//! before the refusal it is checking -- the non-vacuity property the whole binary exists to prove.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A private directory holding one case's transport and the request it received.
pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    pub fn new(label: &str) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "sts2-jev-bridge-process-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)
            .map_err(|error| format!("cannot create the scratch directory: {error}"))?;
        Ok(Self { root })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Writes the executable a case hands to `--transport`.
    ///
    /// The body of `reply` runs after the request has been captured and the marker written, so a
    /// case cannot pass by refusing before the provider answer it names ever arrived.
    pub fn transport(&self, name: &str, reply: &str) -> Result<PathBuf, String> {
        use std::os::unix::fs::PermissionsExt;
        let path = self.path(name);
        let script = format!(
            "#!/bin/sh\ncat > '{}'\nprintf invoked > '{}'\n{reply}\n",
            self.path("request.json").display(),
            self.marker().display()
        );
        fs::write(&path, script).map_err(|error| format!("cannot write the transport: {error}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("cannot make the transport executable: {error}"))?;
        Ok(path)
    }

    pub fn marker(&self) -> PathBuf {
        self.path("invoked")
    }

    /// Writes the executable a case hands to `--transport`, serving `body` on standard output.
    ///
    /// The body travels in its own file rather than inside the shell script, so no case has to
    /// quote a payload through a shell and a valid answer cannot be mangled into an invalid one.
    pub fn transport_serving(&self, name: &str, body: &str) -> Result<PathBuf, String> {
        let answer = self.path(&format!("{name}.body"));
        fs::write(&answer, body)
            .map_err(|error| format!("cannot write the provider answer: {error}"))?;
        self.transport(name, &format!("cat '{}'", answer.display()))
    }

    /// The exact bytes the transport received, once a case has run one to completion.
    pub fn captured_request(&self) -> Result<Vec<u8>, String> {
        fs::read(self.path("request.json"))
            .map_err(|error| format!("the transport received no request: {error}"))
    }

    pub fn invoked(&self) -> bool {
        self.marker().exists()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
