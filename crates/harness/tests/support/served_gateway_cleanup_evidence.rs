// SPDX-License-Identifier: MIT

//! The #556 half of `served_gateway_stderr_evidence`: a gateway that exited non-zero **on its
//! own** must be reported with its own bytes attached, under its own label, and persisted.
//!
//! Refs sts2-harness#556. This is the branch the seven remaining `served/*` sites share, and it
//! is the branch #548 could not reach. #548's stub gateway makes the served scenario fail
//! *before* the cleanup check, so the pre-existing tests never executed the lines below. The
//! stub here is a different shape of gateway — one that answers its request and then leaves by
//! itself — so the scenario finishes its own work, observes a gateway that exited with a
//! non-zero code and no signal, and takes the cleanup branch.
//!
//! It is a separate file rather than a shorter test in the parent because the stub is the
//! substance of the test, and because the parent must stay inside the repository's preferred
//! test-file size budget.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

use super::{CLEANUP_MARKER, process, run_in_child};

/// The exit code the cleanup stub leaves behind. Any non-zero code with **no signal** takes the
/// cleanup-failure branch; this one is an ordinary non-zero exit and nothing in the branch under
/// test inspects its value.
const CLEANUP_EXIT_CODE: u8 = 9;

/// The label the reached site persists under, stated here as this test's own literal rather than
/// imported from the composition, so a rename in the implementation cannot silently follow the
/// assertion and keep it passing. It is the `wrong-instance` negative case's own label, which
/// is what the peer-acceptance step persists under.
const CLEANUP_CASE_LABEL: &str = "wrong-instance";

/// A clean teardown must stay clean: a SIGKILLed gateway is not a cleanup failure.
///
/// `stop` SIGKILLs the gateway's process group, so *every* healthy served scenario leaves
/// `signal() == Some(9)`. If the helper treated that as a failure, all eight sites would report
/// on every successful run — so the passing half of the condition is asserted directly, from a
/// synthesised status rather than by waiting on a real teardown. Together with the child-process
/// test below this pins both directions: a signal is a pass, and a signal-free non-zero exit is
/// the only failure.
#[test]
fn a_sigkilled_gateway_is_not_a_cleanup_failure() -> Result<(), Box<dyn std::error::Error>> {
    assert!(
        process::gateway_cleanup_failure(CLEANUP_CASE_LABEL, &output_signaled(9)).is_ok(),
        "a gateway the lane itself SIGKILLed is a clean teardown, not a failure; reporting it \
         would make every served step fail (sts2-harness#556)."
    );
    assert!(
        process::gateway_cleanup_failure(CLEANUP_CASE_LABEL, &output_exited(0)).is_ok(),
        "a gateway that exited zero on its own is a clean teardown, not a failure \
         (sts2-harness#556)."
    );
    Ok(())
}

/// A gateway that exited non-zero with **no signal** is the only cleanup failure.
///
/// This is the branch the seven remaining sites share, asserted through the same helper the
/// sites call, and it is stated as this test's own literals rather than by importing the
/// composition's condition — so a change to the rule has to fail here deliberately.
#[test]
fn a_signal_free_non_zero_gateway_exit_is_a_cleanup_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let error = process::gateway_cleanup_failure(
        CLEANUP_CASE_LABEL,
        &output_exited(i32::from(CLEANUP_EXIT_CODE)),
    )
    .expect_err("a signal-free non-zero exit must take the cleanup-failure branch");
    let error = error.to_string();
    assert!(
        error.contains(&format!("{CLEANUP_CASE_LABEL}: gateway cleanup failed")),
        "the failure must name the scenario under its own label, or the reader cannot tell \
         which scenario the streams belong to (sts2-harness#556). It was: {error}"
    );
    assert!(
        error.contains(&format!("gateway_stderr={CLEANUP_MARKER}")),
        "the failure must carry the gateway's own streams in band (sts2-harness#556). \
         It was: {error}"
    );
    Ok(())
}

/// An [`Output`] whose gateway **exited** with `code`, carrying the marker on its stderr.
///
/// Synthesised rather than obtained from a real teardown so the condition can be tested at
/// both ends without depending on process scheduling under a contended host. `code == 9` here
/// means *exit* 9 with no signal, which is the failure shape — it is not the SIGKILL that a
/// clean teardown produces, and [`output_signaled`] is the one that models that.
fn output_exited(code: i32) -> Output {
    use std::os::unix::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(code << 8),
        stdout: Vec::new(),
        stderr: format!("{CLEANUP_MARKER}\n").into_bytes(),
    }
}

/// An [`Output`] whose gateway was **killed** by `signal`, carrying the marker on its stderr.
///
/// This is what a clean teardown leaves behind, and the reason the branch has to accept it: a
/// gateway the lane itself killed reported nothing, so calling that a failure would fire on
/// every healthy run.
fn output_signaled(signal: i32) -> Output {
    use std::os::unix::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(signal),
        stdout: Vec::new(),
        stderr: format!("{CLEANUP_MARKER}\n").into_bytes(),
    }
}

/// A gateway that exited non-zero on its own must be reported with its own bytes attached.
///
/// The message and the persisted file are asserted together because either alone is
/// unfalsifiable: the message proves the label survived, and the label is what the persisted
/// file name is derived from, so together they are what makes the bytes findable again.
#[test]
fn a_gateway_that_died_on_its_own_is_reported_with_its_own_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = process::TempDir::new()?;
    let evidence = temporary.path.join("cleanup-evidence");
    let report = run_in_child("cleanup", Some(&evidence))?;

    assert!(
        report.contains(&format!("{CLEANUP_CASE_LABEL}: gateway cleanup failed")),
        "a gateway that exited non-zero without a signal must be reported as a cleanup failure \
         under the scenario's own label, or a reader cannot tell which scenario the bytes \
         belong to (sts2-harness#556). The report was: {report}"
    );
    assert!(
        report.contains(&format!("gateway_stderr={CLEANUP_MARKER}")),
        "the cleanup failure must carry the gateway's own stderr in band; a bare \
         `gateway cleanup failed` leaves a gateway that died on its own unattributable \
         (sts2-harness#556). The report was: {report}"
    );

    // Asserted as an exact file name rather than a `contains` over the directory: the collision
    // this guards against is two scenarios in one lane step overwriting each other, which is
    // invisible to a content-only check. The stem is the sanitised form of the full failure
    // context, so only the case's own label keeps its dashes and the rest is joined by
    // underscores; spelling it out here, rather than reusing the implementation's sanitiser, is
    // the point — a rename on either side has to fail this assertion instead of following it.
    let persisted = evidence.join(format!(
        "gateway-{CLEANUP_CASE_LABEL}_gateway_cleanup_failed_exit_status_\
         {CLEANUP_EXIT_CODE}.stderr"
    ));
    assert!(
        persisted.is_file(),
        "the cleanup failure persisted nothing under its own label, so the lane's failure-only \
         dump step still prints nothing for this step (sts2-harness#556). It looked for {} and \
         the directory held {:?}",
        persisted.display(),
        read_dir_names(&evidence)?
    );
    assert_eq!(
        fs::read_to_string(&persisted)?.trim(),
        CLEANUP_MARKER,
        "the persisted file must hold the gateway's own bytes, not a report about them \
         (sts2-harness#556)."
    );
    Ok(())
}

/// A stub gateway that serves exactly one request and then exits non-zero **on its own**.
///
/// This is the shape the cleanup branch exists for, and it is not the #548 shape. A clean
/// teardown SIGKILLs the gateway's process group, so a healthy gateway leaves
/// `signal() == Some(9)` and never takes the branch at all; the branch can only ever fire for a
/// gateway that exited by itself with a non-zero code and no signal. That is why this stub
/// cannot simply be the #548 one: exiting early fails the scenario at `ready` and at the serve
/// step, and it never reaches the cleanup check the issue is about.
///
/// So it binds the port, answers a single connection, and *then* leaves — after the scenario
/// has otherwise finished with it. `ready` sees a bound socket, and the non-zero exit is
/// observed only by the cleanup check, where it is the sole failure.
pub(super) fn stub_gateway_exiting_non_zero(
    directory: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("stub-gateway-cleanup.sh");
    let exit_code = CLEANUP_EXIT_CODE.to_string();
    fs::write(
        &path,
        format!(
            // As in the parent's `stub_gateway`, each Python line is its own `format!` element:
            // a `\`-continued Rust string would strip the following lines' indentation.
            "{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
            "#!/bin/sh\n",
            "printf '%s\\n' '",
            CLEANUP_MARKER,
            "' >&2\n",
            "exec python3 - <<'PY'\n",
            "import os, socket, sys\n",
            "addr = os.environ[\"STS2_GATEWAY_ADDR\"]\n",
            "host, _, port = addr.rpartition(\":\")\n",
            "listener = socket.socket()\n",
            "listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n",
            "listener.bind((host, int(port)))\n",
            "listener.listen(8)\n",
            "connection, _ = listener.accept()\n",
            "connection.close()\n",
            "sys.exit(",
            &exit_code,
            ")\nPY\n",
        ),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

/// Every file name under `root`, for an assertion that has to explain what it did find.
fn read_dir_names(root: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(root)? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}
