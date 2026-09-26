// SPDX-License-Identifier: MIT

//! The gateway-evidence *naming* and *placement* rules, pinned separately from the
//! served-report tests. Refs #548.
//!
//! Split from `served_gateway_stderr_evidence.rs` so both files stay inside the repository's
//! preferred test-file size budget. The stubs and markers that actually drive a served failure
//! live in that file; nothing here spawns a gateway or a service, so nothing here depends on
//! the host's load or on the pinned peer's behaviour.

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
/// No served composition may re-hand-roll the teardown check.
///
/// The seven unwrapped sites in #556 were not an oversight in one file; they were eight
/// hand-written copies of one condition, and the copies drifted. #548 fixed the copy it
/// happened to touch and the other seven kept the old shape, with nothing to fail. So
/// `gateway_teardown_failure` is now the only spelling of that condition, and this pins that
/// structurally: a `served` file that writes the predicate out again — in any polarity, with
/// any status field — fails here instead of quietly re-opening the #556 gap.
///
/// It reads the repository's own sources the way the policy gate does. It is a companion to,
/// not a substitute for, the behavioural test in `served_gateway_stderr_evidence.rs`: that one
/// proves the helper carries the bytes, and this one proves nobody bypasses the helper.
#[test]
fn no_served_composition_hand_rolls_the_teardown_check() -> Result<(), Box<dyn std::error::Error>> {
    let served = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support/runtime_v4_executable_composition_process/served");
    let mut entries = std::fs::read_dir(&served)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|value| value == "rs"))
        .collect::<Vec<_>>();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "no served sources were read under {}, so this check proved nothing",
        served.display()
    );
    let offenders = entries
        .iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(path).ok()?;
            // `assert_killed` checks the *workflow service* was killed, not the gateway, and
            // reads `output` rather than `gateway_output`. Matching on the gateway's own
            // binding is what keeps this check from flagging that unrelated correct predicate.
            let hand_rolled = source
                .lines()
                .filter(|line| {
                    let trimmed = line.trim_start();
                    trimmed.contains("gateway_output.status.code()")
                        || trimmed.contains("gateway_output.status.signal()")
                })
                .count();
            (hand_rolled > 0).then(|| format!("{}: {hand_rolled}", path.display()))
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "a served composition re-spelled the gateway teardown condition instead of calling \
         `gateway_teardown_failure`, so it can drop the gateway's streams again \
         (sts2-harness#556). Files: {offenders:?}"
    );
    Ok(())
}

/// The evidence files must land where the lane's failure-only dump step can see them.
///
/// That step runs `find "$RUNNER_TEMP/runtime-peer-contract" -type f -maxdepth 2`, so a file
/// written anywhere deeper is invisible no matter how correct its contents are.
///
/// The risk here is *silent*: `find -maxdepth 2` matching nothing still exits 0, so a writer
/// that nested its output one level too deep would leave a green lane printing no evidence at
/// all. That is the same shape as the original #548 defect, so the placement is driven through
/// the real writer rather than asserted by reading the helper's arithmetic.
///
/// `STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR` is process-wide and this workspace forbids
/// `unsafe`, so the write is exercised in a child process exactly as
/// `served_gateway_stderr_evidence.rs` does it. The child is given the directory layout the
/// workflow hands its served steps — `<tmp>/runtime-peer-contract/<per-step>` — which is what
/// makes the `find` root and the per-step directory the same relative path CI uses.
#[test]
fn gateway_streams_are_written_at_the_depth_the_dump_step_reads()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = process::TempDir::new()?;
    let dump_root = temporary.path.join("runtime-peer-contract");
    let per_step = dump_root.join("served-policy-gate");
    // The child's exit status carries the result: the child asserts, the parent relays.
    let status = write_streams_in_child(&per_step)?;
    assert!(
        status.success(),
        "the child that drives the real writer exited {status}, so the depth assertions below \
         never ran and this test would have passed without checking anything"
    );

    let mut files = Vec::new();
    collect_files(&dump_root, &mut files)?;
    assert!(
        !files.is_empty(),
        "the writer produced nothing under {}, so the lane's dump step would print no evidence \
         for a served step at all",
        dump_root.display()
    );
    // Exactly depth 2 relative to the `find` root: `<root>/<per-step>/<file>`. A file one level
    // deeper is invisible to `-maxdepth 2`; a file at the root instead means the per-step
    // directory the workflow names is not the one actually being written.
    for file in files {
        assert!(
            file.parent() == Some(per_step.as_path()),
            "evidence file {} is not at depth 2 under the lane's find root, so `find -type f \
             -maxdepth 2` will not print it even though it exists",
            file.display()
        );
    }
    Ok(())
}

/// Env var marking the depth child. Absent in an ordinary run, so the child is not entered
/// when the whole binary is executed by `cargo test`.
const CHILD: &str = "STS2_DEPTH_CHILD";
/// Env var carrying the per-step evidence directory into the depth child.
const DEPTH_DIR: &str = "STS2_DEPTH_EVIDENCE_DIR";

/// Run [`gateway_streams_depth_child`] in a child process and return its exit status.
fn write_streams_in_child(per_step: &Path) -> Result<std::process::ExitStatus, std::io::Error> {
    std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", "gateway_streams_depth_child", "--nocapture"])
        .env(CHILD, "1")
        .env(DEPTH_DIR, per_step)
        // The lane variable is what `gateway_failure_evidence` actually reads, so this is the
        // real CI wiring, not a simulation of it. Without it the helper is a no-op on the
        // write path and the child exits 0 having written nothing — which is exactly the
        // #548 defect this check exists to catch, and exactly what the assertion reported.
        .env("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR", per_step)
        .status()
}

/// The child half of the depth check: drive the real writer into `per_step`.
///
/// It exits non-zero rather than asserting, because the parent process owns the assertions
/// that matter: a panic here would still fail the run, but an explicit status makes the
/// "the child never ran" case distinguishable from "the child ran and the placement is wrong".
#[test]
fn gateway_streams_depth_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let per_step = match std::env::var_os(DEPTH_DIR) {
        Some(value) => PathBuf::from(value),
        None => {
            eprintln!("{CHILD} ran without {DEPTH_DIR}");
            std::process::exit(2);
        }
    };
    // A deliberately awkward label: a slash, a space, a newline, and a tail far past the
    // 120-character cap. The writer must sanitise it, and the placement check above then
    // measures real sanitised output rather than a trivially legal file name.
    let label = format!(
        "served-policy-gate: nested/path with spaces\nand a long tail {}",
        "x".repeat(200)
    );
    let gateway = std::process::Output {
        status: std::process::ExitStatus::default(),
        stdout: b"gateway stdout marker".to_vec(),
        stderr: b"gateway stderr marker".to_vec(),
    };
    let error = process::gateway_failure_evidence(&label, &gateway);
    if !error.to_string().contains("gateway stderr marker") {
        eprintln!("the writer did not attach the gateway's stderr: {error}");
        std::process::exit(3);
    }
    // The helper is a deliberate no-op on the write path when the evidence variable is unset,
    // so attaching the streams in-band says nothing about having persisted them. Assert the
    // bytes landed here, and exit non-zero otherwise, so the parent's `status.success()` is a
    // real control rather than a rubber stamp on a child that quietly did nothing.
    let mut written = Vec::new();
    if let Err(error) = collect_files(&per_step, &mut written) {
        eprintln!("could not read {}: {error}", per_step.display());
        std::process::exit(6);
    }
    if written.is_empty() {
        eprintln!("the writer persisted nothing under {}", per_step.display());
        std::process::exit(4);
    }
    let mut persisted = String::new();
    for file in &written {
        match std::fs::read_to_string(file) {
            Ok(contents) => persisted.push_str(&contents),
            Err(error) => eprintln!("could not read back {}: {error}", file.display()),
        }
    }
    if !persisted.contains("gateway stderr marker") {
        eprintln!("the persisted evidence did not carry the gateway's stderr: {persisted}");
        std::process::exit(5);
    }
    std::process::exit(0);
}
