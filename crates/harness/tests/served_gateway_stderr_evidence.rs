// SPDX-License-Identifier: MIT

//! Pin that a `served_*` failure carries the gateway's own stderr, and that a served
//! scenario persists it where the lane's failure-only dump step can read it. Refs #548.
//!
//! The served compositions spawn the gateway with piped stdout/stderr and read those streams
//! back out of `stop`, but until #548 they dropped them: the only consumer of the gateway's
//! stderr was `write_evidence`, which no served scenario calls, and no served lane step named
//! an evidence directory. A gateway refusal therefore never reached a served failure, and the
//! `served/*` error strings carried a bare `stderr=` label holding the *workflow service's*
//! bytes, so a reader saw something that looked like gateway output and reasonably concluded
//! the gateway had reported when it had not.
//!
//! # Shape of the test
//!
//! Each test spawns **this same test binary** as a child, with an env var naming which case to
//! run, and reads the child's standard error — the same shape
//! `exo_supervised_child_diagnostic.rs` uses. The child calls the real
//! [`process::run_served_policy_gate`] against a stub gateway and a stub workflow-service
//! binary and prints the failure text it received; the parent asserts on that text.
//!
//! A child process is used rather than a direct call for one concrete reason: the evidence
//! directory is selected by `STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR`, a *process-wide*
//! variable, and this workspace forbids `unsafe`, so there is no way to set it for one test
//! without also setting it for its siblings. Each child therefore gets its own environment,
//! and the tests remain safe to run in parallel. It is the same reason the existing supervised
//! diagnostic suite uses a child.
//!
//! The stub gateway writes a recognisable marker to its own stderr and then exits, so `ready`
//! observes the exit inside the served scenario and the scenario fails *on the served path* —
//! the only place the #548 change lives. That is the exact shape of #541: the gateway refused
//! or died, the served path reported a failure, and the gateway's own explanation was
//! dropped. Asserting against a stub rather than the pinned peer is deliberate: the claim is
//! about this repository's plumbing, and an assertion against the real gateway would pass
//! vacuously whenever the peer is silent — the condition #541 observed, and the reason its
//! flake could never be reproduced.
//!
//! None of these tests is `#[ignore]`d, none needs an operator-built peer binary, and none
//! depends on an execution count, so they run in an ordinary `cargo test` and cannot pass by
//! being renamed.

#![cfg(unix)]

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
// This test binary compiles the shared fixture and process support but exercises only the
// served policy scenario, so several `FixtureMode` variants and fixture helpers are unused
// *here* while remaining live in `runtime_v4_executable_composition`. The process support
// module carries its own crate-level `allow(dead_code)`, so this suppression is only needed
// for the fixture, and it is scoped to this binary rather than added to the shared file so
// that other binary keeps its own dead-code analysis.
#[allow(dead_code)]
mod fixture;
#[path = "support/runtime_v4_executable_composition_process.rs"]
mod process;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Env var naming the case the child process should run. Absent in an ordinary run, so each
/// helper below is a no-op unless a parent test launched this binary deliberately.
const CASE: &str = "STS2_TEST_SERVED_GATEWAY_STDERR_CASE";

/// The recognisable line the stub gateway writes to its own stderr. Chosen to be a marker,
/// not prose: a real gateway refusal names a header, and the claim under test is that
/// *whatever the gateway says* survives the served path, not that this sentence appears.
const GATEWAY_MARKER: &str = "sts2-harness-548-stub-gateway-stderr-marker";

/// The stub workflow service's own marker. Both markers appear in one report, each under its
/// own label, which is what makes the attribution checkable rather than merely present.
const SERVICE_MARKER: &str = "sts2-harness-548-stub-service-stderr-marker";

/// A served scenario must report the gateway's stderr when it fails.
///
/// Acceptance criterion 1. On `main` the served path discards the gateway's bytes, so
/// `GATEWAY_MARKER` is absent from the report and this assertion fails.
#[test]
fn a_served_scenario_reports_the_gateway_stderr_it_captured()
-> Result<(), Box<dyn std::error::Error>> {
    let report = run_in_child("report", None)?;
    assert!(
        report.contains(GATEWAY_MARKER),
        "the served path dropped the gateway's stderr, so a gateway refusal stays \
         unattributable (sts2-harness#548). The served report was: {report}"
    );
    assert!(
        report.contains("gateway_stderr="),
        "the served report must label the stream as the gateway's, not as an unqualified \
         `stderr=` a reader cannot attribute (sts2-harness#548). The served report was: {report}"
    );
    Ok(())
}

/// The `served/*` error strings must stop implying they carry gateway output.
///
/// Acceptance criterion 3. Each interpolated the *service's* bytes behind a bare `stderr=`,
/// which reads as though the gateway had reported. The label is now qualified, and this
/// checks the qualification on a real served failure rather than by grepping the sources.
#[test]
fn a_served_scenario_labels_the_service_stream_so_it_cannot_be_read_as_the_gateway()
-> Result<(), Box<dyn std::error::Error>> {
    let report = run_in_child("report", None)?;
    assert!(
        report.contains(&format!("service_stderr={SERVICE_MARKER}")),
        "the served report must qualify the workflow service's own stream, and the \
         qualification must sit directly on the bytes it describes (sts2-harness#548). \
         The served report was: {report}"
    );
    assert!(
        !bare_stderr_label_carries(&report, SERVICE_MARKER),
        "a bare `stderr=` label carrying the workflow service's bytes is exactly what told \
         readers the gateway had reported when it had not (sts2-harness#548). The served \
         report was: {report}"
    );
    assert!(
        report.contains(&format!("gateway_stderr={GATEWAY_MARKER}")),
        "each stream must be attributed, and the gateway's marker must sit directly on \
         `gateway_stderr=`. The served report was: {report}"
    );
    Ok(())
}

/// The same bytes must be persisted where the lane's failure-only dump step reads them.
///
/// Acceptance criterion 2. The `served_*` lane steps now each name an evidence directory,
/// but without a writer on the served path that directory stayed empty and
/// *"Show owned-process diagnostics"* printed nothing for a served step.
#[test]
fn a_served_scenario_persists_its_gateway_streams_under_the_evidence_directory()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = process::TempDir::new()?;
    let evidence = temporary.path.join("served-evidence");
    run_in_child("report", Some(&evidence))?;

    // Read the directory back the way the lane's dump step does: every file under it.
    let mut files = Vec::new();
    collect_files(&evidence, &mut files)?;
    assert!(
        !files.is_empty(),
        "the served path wrote nothing under {}, so the lane's diagnostic dump would print \
         nothing for a served step (sts2-harness#548)",
        evidence.display()
    );
    let mut persisted = String::new();
    for file in files {
        persisted.push_str(&fs::read_to_string(&file)?);
    }
    assert!(
        persisted.contains(GATEWAY_MARKER),
        "the persisted evidence did not carry the gateway's own stderr, so a reader following \
         the lane's dump step still cannot attribute a refusal (sts2-harness#548). It carried: \
         {persisted}"
    );
    Ok(())
}

/// Launch this test binary as a child running `case`, and return the failure report it
/// printed on its standard error.
///
/// `evidence` names the directory the child hands to the served path through
/// `STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR`; `None` leaves it unset, which is the state
/// every served lane step was in before #548 and the state the two report tests need.
fn run_in_child(case: &str, evidence: Option<&Path>) -> Result<String, Box<dyn std::error::Error>> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", "served_gateway_stderr_child", "--nocapture"])
        .env(CASE, case);
    if let Some(root) = evidence {
        command.env("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR", root);
    }
    let output = command.output()?;
    let report = String::from_utf8_lossy(&output.stderr).into_owned();
    if !report.contains(GATEWAY_MARKER) {
        return Err(format!(
            "the child served scenario did not reach the assertion under test. The child exited \
             with {} and printed: {report}",
            output.status
        )
        .into());
    }
    Ok(report)
}

/// The child half: run the real served scenario against the stubs and print the failure.
///
/// A no-op unless a parent launched this binary with [`CASE`] set, so it costs an ordinary
/// `cargo test` nothing. The scenario is *expected* to fail — that is the subject — so this
/// prints the failure and exits successfully; the parent is what decides whether the failure
/// carried the right bytes.
#[test]
fn served_gateway_stderr_child() {
    let Some(case) = std::env::var_os(CASE) else {
        return;
    };
    // `var_os` yields an `OsString`, which has no `Display`; the parent always sets a plain
    // UTF-8 case name, so lossy conversion is exact here and keeps the diagnostic readable.
    let case_name = case.to_string_lossy();
    let temporary = match process::TempDir::new() {
        Ok(temporary) => temporary,
        Err(error) => {
            eprintln!("child could not create its temporary directory: {error}");
            return;
        }
    };
    let gateway_stub = match stub_gateway(&temporary.path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("child could not write its gateway stub: {error}");
            return;
        }
    };
    let service_stub = match stub_service(&temporary.path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("child could not write its service stub: {error}");
            return;
        }
    };
    match process::run_served_policy_gate(&gateway_stub, &service_stub, &service_stub) {
        Ok(()) => eprintln!("{case_name}: the served scenario unexpectedly succeeded"),
        Err(error) => eprintln!("{case_name}: {error}"),
    }
}

/// True when `report` places `marker` immediately behind a **bare** `stderr=` label.
///
/// A plain `contains("stderr=…")` cannot express this: the qualified `service_stderr=` label
/// ends with the same three characters, so every correctly qualified report would match the
/// string it is meant to be exempt from. This inspects the character *before* `stderr=`,
/// which is exactly the distinction a reader is being asked to make.
fn bare_stderr_label_carries(report: &str, marker: &str) -> bool {
    let needle = format!("stderr={marker}");
    let mut from = 0;
    while let Some(offset) = report[from..].find(&needle) {
        let at = from + offset;
        let qualified = at > 0
            && report[..at]
                .chars()
                .next_back()
                .is_some_and(|value| value.is_alphanumeric() || value == '_');
        if !qualified {
            return true;
        }
        from = at + needle.len();
    }
    false
}

/// Every regular file under `root`, so the assertions read the evidence directory the way the
/// lane's dump step reads it rather than assuming a file name.
fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
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
