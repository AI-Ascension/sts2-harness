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

    /// Writes a transport that refuses **without reading its standard input**.
    ///
    /// [`Scratch::transport`] opens with `cat > request.json`, so it always drains stdin before it
    /// refuses. That makes the bridge's writer succeed and hides the whole class of refusal this
    /// exists for: a transport that declines a request and exits without consuming it, which is
    /// what an operator transport does when it rejects the request before reading the body.
    ///
    /// Without it, `write_all` fails with `EPIPE` and the cause a caller sees is a plumbing artifact
    /// rather than the refusal (Refs #751). The marker is still written, so `invoked()` reports that
    /// the transport ran and the case cannot pass vacuously.
    pub fn transport_refusing_without_reading(
        &self,
        name: &str,
        reply: &str,
    ) -> Result<PathBuf, String> {
        use std::os::unix::fs::PermissionsExt;
        let path = self.path(name);
        let script = format!(
            "#!/bin/sh\nprintf invoked > '{}'\n{reply}\n",
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

    /// The bridge's own request, padded past the pipe buffer.
    ///
    /// A pipe holds 64 KiB by default on Linux, so the ordinary request -- about 300 bytes -- is
    /// fully absorbed before the child can exit and the bridge's `write_all` never fails. This is
    /// the same request with only its size changed, so everything the bridge validates before the
    /// exchange is identical.
    ///
    /// The padding is bounded rather than maximal on purpose. Measured against `main` at
    /// `75260418`, the bridge accepts this request up to roughly 64 KiB and reports `state and
    /// question exceed the budget` above it, while the pipe buffer is 64 KiB -- the two bounds
    /// nearly coincide. 62_000 sits inside the window with room on both sides: large enough that
    /// the writer is still writing when the transport exits, small enough that the bridge accepts
    /// the request at all. A value at either edge would fail for a reason unrelated to the refusal.
    pub fn request_past_the_pipe_buffer() -> Vec<u8> {
        let padding = "x".repeat(62_000);
        serde_json::to_vec(&serde_json::json!({
            "model_execution_id": "model-execution-7",
            "objective": format!("survive the turn {padding}"),
            "hard_constraints": ["never end the turn with unspent lethal"],
            "legal_action_ids": ["play:card-17", "play:card-18", "combat.end-turn"],
            "observation": {
                "state_id": "combat-1",
                "generation": 3,
                "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
                "state": {"state": "combat", "turn_index": 2},
            },
        }))
        .unwrap_or_default()
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
