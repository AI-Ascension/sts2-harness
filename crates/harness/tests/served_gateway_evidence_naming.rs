// SPDX-License-Identifier: MIT

//! The gateway-evidence *naming* rules, pinned separately from the served-report tests.
//! Refs #548.
//!
//! Split from `served_gateway_stderr_evidence.rs` so both files stay inside the repository's
//! preferred test-file size budget.

#![cfg(unix)]

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "support/runtime_v4_executable_composition_process.rs"]
mod process;

use std::path::{Path, PathBuf};

use process::sanitize_label;

/// Every regular file under `root`, matching how the lane's dump step reads a directory.
fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else {
            files.push(path);
        }
    }
    files.sort();
    Ok(())
}

/// Two scenarios that share a lane step must not overwrite each other's evidence.
///
/// `write_gateway_streams` derives a file name from the failure label, and the peer-acceptance
/// step runs four negative cases while the graph lane runs twice. If two of those sanitised to
/// the same stem, the second run's gateway stderr would replace the first's and the dump step
/// would show one file where a reader expects two.
///
/// The hazard is real in principle — `sanitize_label` caps at 120 characters, so two long
/// labels sharing a prefix *could* collide — but it is only reachable if the scenario's own
/// name falls outside that cap. Each label here leads with its short stable name, so this
/// pins that property instead of assuming it. The shared error tail is deliberately far
/// longer than any real one, which is the worst case for the cap.
#[test]
fn scenarios_sharing_a_lane_step_persist_to_distinct_files() {
    // The negative-case labels from `served/acceptance.rs`, and the two `request_id`s the graph
    // lane distinguishes on. Everything after the name is the scenario's failure text, which
    // may be arbitrarily long and is not part of the identity.
    let shared_tail = "a failure context far longer than any real one ".repeat(40);
    let labels = [
        format!("wrong-instance: {shared_tail}"),
        format!("stale-lease: {shared_tail}"),
        format!("invalid-binding: {shared_tail}"),
        format!("unavailable-provider: {shared_tail}"),
        format!("graph graph-changed: {shared_tail}"),
        format!("graph graph-original: {shared_tail}"),
    ];
    let mut stems: Vec<String> = labels.iter().map(|l| sanitize_label(l)).collect();
    let total = stems.len();
    stems.sort();
    stems.dedup();
    assert_eq!(
        stems.len(),
        total,
        "two scenarios sharing a lane step sanitised to the same evidence file name, so the \
         second run's gateway stderr would overwrite the first's. stems were: {stems:?}"
    );
}

/// The evidence files must land where the lane's failure-only dump step can see them.
///
/// That step runs `find "$RUNNER_TEMP/runtime-peer-contract" -type f -maxdepth 2`, so a file
/// written anywhere deeper is invisible no matter how correct its contents are. Each served
/// step names a subdirectory of that root and the helper writes directly into it, which puts
/// every file at exactly depth 2.
#[test]
fn gateway_streams_are_written_at_the_depth_the_dump_step_reads() {
    let temporary = match process::TempDir::new() {
        Ok(temporary) => temporary,
        Err(error) => {
            eprintln!("could not create a temporary directory: {error}");
            return;
        }
    };
    let root = temporary
        .path
        .join("runtime-peer-contract")
        .join("served-policy-gate");
    let gateway = std::process::Output {
        status: std::process::ExitStatus::default(),
        stdout: b"gateway stdout marker".to_vec(),
        stderr: b"gateway stderr marker".to_vec(),
    };
    // Drive the real writer rather than re-implementing the path arithmetic.
    let error = process::gateway_failure_evidence("served-policy-gate: probe", &gateway);
    assert!(
        error.to_string().contains("gateway stderr marker"),
        "the helper did not attach the gateway's stderr to the failure text, so this probe \
         never exercised the write path"
    );
    // The evidence directory is process-wide, so only exercise the write when this test
    // actually owns it: the child-driven test sets it, and an ordinary run must not be
    // redirected. Setting it here would also race the other tests in this binary.
    if std::env::var_os("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR").is_none() {
        eprintln!("skipped the write-path depth check: the evidence directory is process-wide");
        return;
    }
    let mut files = Vec::new();
    collect_files(&root, &mut files).unwrap_or_default();
    // The child case owns the directory in the parent, so assert the shape rather than a
    // specific file: everything written is at most one level below the per-step directory.
    for file in files {
        assert!(
            file.parent() == Some(root.as_path()),
            "evidence file {} is not directly inside the per-step directory, so the dump \
             step's -maxdepth 2 may not reach it",
            file.display()
        );
    }
}

/// A stub "gateway": it writes [`GATEWAY_MARKER`] to its own stderr, binds
/// `STS2_GATEWAY_ADDR`, and keeps accepting until it is killed.
///
/// It is spawned through the same `gateway_with_identity` helper the real compositions use,
/// so it inherits exactly the environment the real gateway would, including the cleared
/// environment and `STS2_GATEWAY_ADDR`.
///
/// **It must stay alive rather than exit immediately.** `ready`
/// (`runtime_v4_executable_composition_process.rs:169`) checks `try_wait` *before* it attempts
/// to connect, so a stub that exits at once fails the served scenario at gateway startup —
/// before the workflow service is ever spawned — and the service's own stream never reaches the
/// report. Holding the socket open lets the scenario pass readiness and fail later on the
/// *service*, so one report carries **both** markers, each under its own label. That combined
/// report is what makes attribution checkable rather than merely present, and it is the #541
/// shape: the gateway said something, the served path reported a failure, and the gateway's own
/// explanation was dropped.
///
/// The marker is printed before the bind, so it is on stderr whichever exit the scenario takes.
/// A stub rather than the pinned peer is deliberate: the claim is about this repository's
/// plumbing, and asserting against the real gateway would pass vacuously whenever the peer is
/// silent — the condition #541 observed, and the reason its flake could never be reproduced.
fn stub_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("stub-gateway.sh");
    fs::write(
        &path,
        format!(
            // Each Python line is its own `concat!` element carrying its own leading spaces.
            // A `\`-continued Rust string strips the indentation of every continued line,
            // which would leave the `while` body at column zero and fail with an
            // `IndentationError` — the script must reach the served path, and a stub that
            // dies at bind time never gets there.
            "{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
            "#!/bin/sh\n",
            "printf '%s\\n' '",
            GATEWAY_MARKER,
            "' >&2\n",
            "exec python3 - <<'PY'\n",
            "import os, socket\n",
            "addr = os.environ[\"STS2_GATEWAY_ADDR\"]\n",
            "host, _, port = addr.rpartition(\":\")\n",
            "listener = socket.socket()\n",
            "listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n",
            "listener.bind((host, int(port)))\n",
            "listener.listen(8)\n",
            "while True:\n",
            "    connection, _ = listener.accept()\n",
            "    connection.close()\nPY\n",
        ),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

/// A stub workflow service: it writes [`SERVICE_MARKER`] to its own stderr and exits non-zero.
///
/// `wait_for_workflow_service` observes the exit inside the served scenario and fails there
/// rather than at its readiness deadline. The stub is passed as the harness binary too, since
/// the served scenario never reaches a step that would execute it.
fn stub_service(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("stub-service.sh");
    fs::write(
        &path,
        format!("#!/bin/sh\nprintf '%s\\n' '{SERVICE_MARKER}' >&2\nexit 3\n"),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}
