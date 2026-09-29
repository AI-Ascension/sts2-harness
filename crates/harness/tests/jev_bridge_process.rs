// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The System One bridge observed at its own process boundary.
//!
//! The unit suite inside the binary calls `decide` and `record` in process, so it proves a refusal
//! is *returned* but not that the executable turns that refusal into the exit status and the empty
//! standard output the runtime lane reads. These cases run the real binary as a child process.
//!
//! The bridge now performs the exchange itself, so these cases cannot stage a provider answer the
//! way the previous version of this file did. What remains here is the part that is genuinely a
//! process property: the exit status and empty standard output of a refusal, the configuration a
//! `--describe` reports, the refusal of a stale `--transport` configuration, and the credential
//! never appearing in any output. The response classes the provider can return are covered
//! in-process, against bytes, in `jev_tls_transport_tests`.

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Serialises every case that launches the bridge, so this binary never holds more than one
/// bridge alive at a time.
///
/// Each invocation costs a child process running a TLS-capable binary. Left to the default
/// test-thread count, the cases below created those concurrently, and a runner that could not fork
/// answered `EAGAIN`. Serialising keeps the suite's process cost bounded at one bridge.
fn bridge_slot() -> MutexGuard<'static, ()> {
    static SLOT: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = SLOT.get_or_init(|| Mutex::new(()));
    // A panicking case poisons the slot, and the next case must still be able to run: the panic
    // belongs to the case that caused it, not to every case that follows it.
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The one line a refused run prints, on standard error rather than standard output.
const FAILURE_LINE: &str = "System One bridge failed validation or transport";

/// The environment name the operator supplies the provider credential in.
const CREDENTIAL_NAME: &str = "TYPESAFE_API_KEY";

/// A value distinctive enough that any appearance of it in an output is a leak.
const CREDENTIAL_VALUE: &str = "typesafe-credential-8f24c1ab-never-recorded";

/// The bridge's own bound on a request, a response and a decision.
const LIMIT: usize = 128 * 1024;

/// Runs the real bridge once with `body` on standard input.
fn run_with_body(
    body: &[u8],
    environment: &[(&str, &str)],
    arguments: &[&str],
) -> Result<Output, String> {
    let _slot = bridge_slot();
    let mut child = launched(environment, arguments)?
        .spawn()
        .map_err(|error| format!("cannot run the bridge: {error}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or("the bridge has no standard input")?;
        stdin
            .write_all(body)
            .map_err(|error| format!("cannot write the request: {error}"))?;
    }
    child
        .wait_with_output()
        .map_err(|error| format!("cannot collect the bridge output: {error}"))
}

/// Runs the real bridge once with its standard input closed immediately.
///
/// `--describe` returns before reading input, so a case that wrote a request to it would race the
/// child's exit and could fail on a closed pipe rather than on the behaviour under test.
fn run_without_input(environment: &[(&str, &str)], arguments: &[&str]) -> Result<Output, String> {
    let _slot = bridge_slot();
    launched(environment, arguments)?
        .output()
        .map_err(|error| format!("cannot run the bridge: {error}"))
}

fn launched(environment: &[(&str, &str)], arguments: &[&str]) -> Result<Command, String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-jev-bridge"));
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in environment {
        command.env(name, value);
    }
    Ok(command)
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A refused run exits nonzero, prints no decision, and says why.
fn refused_without_a_decision(output: &Output) -> Result<(), String> {
    if output.status.success() {
        return Err(format!(
            "the bridge exited zero and printed {}",
            stdout_of(output)
        ));
    }
    if !output.stdout.is_empty() {
        return Err(format!(
            "a refused run wrote to standard output: {:?}",
            stdout_of(output)
        ));
    }
    if !stderr_of(output).contains(FAILURE_LINE) {
        return Err(format!(
            "the refusal did not name itself: {:?}",
            stderr_of(output)
        ));
    }
    match output.status.code() {
        Some(2) => Ok(()),
        code => Err(format!(
            "a refused run must fail with the bridge's exit status: {code:?}\n{}",
            stderr_of(output)
        )),
    }
}

/// A refusal that never reached the provider is still a refusal at the process boundary.
///
/// The bridge validates the request before it opens a socket, so an oversized request produces the
/// exit status and the empty standard output the runtime lane reads without any network at all.
/// That is what makes this suite runnable offline: the cases below need no provider, no credential,
/// and no reachable host.
#[test]
fn an_oversized_request_is_refused_without_a_decision() -> Result<(), String> {
    let output = run_with_body(&vec![b'x'; LIMIT + 1], &[], &[])?;
    refused_without_a_decision(&output)
}

/// A request whose catalog is empty is refused on the same terms, before any exchange.
#[test]
fn a_request_with_no_legal_action_is_refused_without_a_decision() -> Result<(), String> {
    let empty = serde_json::to_vec(&serde_json::json!({
        "model_execution_id": "model-execution-7",
        "objective": "survive the turn",
        "hard_constraints": ["never end the turn with unspent lethal"],
        "legal_action_ids": [],
        "observation": {
            "state_id": "combat-1",
            "generation": 3,
            "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
            "state": {"state": "combat", "turn_index": 2},
        },
    }))
    .unwrap_or_default();
    let output = run_with_body(&empty, &[], &[])?;
    refused_without_a_decision(&output)
}

/// An unknown option is refused before the bridge reads anything else.
///
/// This is the operator-facing half of the two-digests-into-one change. A configuration written
/// for the previous four-element form must fail loudly at the process boundary rather than have
/// the transport pair ignored, or a run would proceed believing a transport was pinned when the
/// bridge was in fact the only artifact.
#[test]
fn a_stale_transport_argument_is_refused_at_the_process_boundary() -> Result<(), String> {
    let output = run_without_input(&[], &["--transport", "/opt/providers/systemone-transport"])?;
    // A parse failure exits 2 with the usage line rather than the refusal line: the bridge never
    // started a run, so there is no decision to withhold and nothing to explain. What matters is
    // that it exited nonzero, wrote no decision, and named the accepted options -- which no longer
    // include `--transport`.
    if output.status.success() {
        return Err(format!(
            "a stale --transport configuration was admitted: {}",
            stdout_of(&output)
        ));
    }
    if !output.stdout.is_empty() {
        return Err(format!(
            "a refused launch wrote to standard output: {:?}",
            stdout_of(&output)
        ));
    }
    let stderr = stderr_of(&output);
    if !stderr.contains("Usage:") {
        return Err(format!(
            "a refused launch printed no usage line: {stderr:?}"
        ));
    }
    if stderr.contains("--transport") {
        return Err(format!(
            "the usage line still advertises --transport: {stderr:?}"
        ));
    }
    Ok(())
}

/// `--describe` names the in-process transport and the pinned trust anchor.
///
/// A run record has to be able to say what terminated the connection. "It used the system roots"
/// is not a statement a digest-pinned artifact can make, so the description carries the store.
#[test]
fn describe_names_the_in_process_transport_and_its_trust_anchor() -> Result<(), String> {
    let output = run_without_input(&[], &["--describe", "--model", "jev-1.13.0"])?;
    if !output.status.success() {
        return Err(format!("--describe failed: {}", stderr_of(&output)));
    }
    let described: serde_json::Value = serde_json::from_str(&stdout_of(&output))
        .map_err(|error| format!("--describe did not print one object: {error}"))?;
    if described["transport"] != serde_json::json!("in-process") {
        return Err(format!("unexpected transport description: {described}"));
    }
    let store = described["tls_root_store"]
        .as_str()
        .ok_or_else(|| "--describe named no trust anchor".to_owned())?;
    if !store.contains("webpki-roots") {
        return Err(format!(
            "the trust anchor is not the pinned store: {store:?}"
        ));
    }
    Ok(())
}

/// The credential is read by name from the environment and never reaches any output.
///
/// The bridge no longer hands the credential to a second process, so the surface here is its own:
/// the record, the description, and the refusal text. None of them may carry the value.
#[test]
fn the_credential_never_reaches_a_decision_a_description_or_a_refusal() -> Result<(), String> {
    let credential = [(CREDENTIAL_NAME, CREDENTIAL_VALUE)];
    let described = run_without_input(&credential, &["--describe"])?;
    if !described.status.success() {
        return Err(format!("--describe failed: {}", stderr_of(&described)));
    }
    for (where_, text) in [
        ("the description", stdout_of(&described).as_str()),
        (
            "the bridge's standard error",
            stderr_of(&described).as_str(),
        ),
    ] {
        if text.contains(CREDENTIAL_VALUE) {
            return Err(format!("the credential value appeared in {where_}"));
        }
    }
    // A real request reaches the validation and refusal path, which is the one that writes a
    // diagnostic. The bridge refuses an empty catalog before it opens a socket, so this needs
    // neither a provider nor a reachable host.
    let refused = run_with_body(
        &serde_json::to_vec(&serde_json::json!({
            "model_execution_id": "model-execution-7",
            "objective": "survive the turn",
            "hard_constraints": [],
            "legal_action_ids": [],
            "observation": {
                "state_id": "combat-1",
                "generation": 3,
                "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
                "state": {"state": "combat", "turn_index": 2},
            },
        }))
        .unwrap_or_default(),
        &credential,
        &[],
    )?;
    refused_without_a_decision(&refused)?;
    if stderr_of(&refused).contains(CREDENTIAL_VALUE) {
        return Err(String::from("a refusal printed the credential value"));
    }
    Ok(())
}

/// A credential that is missing or cannot go in a header is refused before anything is contacted.
///
/// The exchange is refused, not downgraded and not retried, and -- more importantly -- it is
/// refused *before* a socket is opened, so a misconfigured run never puts a request in front of
/// the provider at all. The cases are driven at the process boundary because the credential comes
/// from the environment, and a refusal that names the reason without echoing the value is
/// asserted for each.
#[test]
fn a_missing_or_header_unsafe_credential_is_refused_without_contacting_the_provider()
-> Result<(), String> {
    let request = serde_json::to_vec(&serde_json::json!({
        "model_execution_id": "model-execution-7",
        "objective": "survive the turn",
        "hard_constraints": [],
        "legal_action_ids": ["act-1"],
        "observation": {
            "state_id": "combat-1",
            "generation": 3,
            "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
            "state": {"state": "combat", "turn_index": 2},
        },
    }))
    .unwrap_or_default();

    // An absent credential. The child inherits this process's environment, so a run that simply
    // does not set the variable is the case: `credential` reads it by name and refuses when it
    // is not there, rather than sending an empty `Bearer ` header and letting the provider answer.
    let refused = run_with_body(&request, &[], &[])?;
    refused_without_a_decision(&refused)?;
    if !stderr_of(&refused).contains("credential") {
        return Err(format!(
            "a run with no credential was refused without naming the credential: {:?}",
            stderr_of(&refused)
        ));
    }

    for (value, expected) in [
        ("", "the provider credential is empty"),
        // A credential carrying CRLF would let it append a header line of its own. Refusing it
        // is the point: escaping it would send something the provider cannot authenticate.
        ("abc\r\nX-Injected: 1", "U+000D"),
        ("abc\ndef", "U+000A"),
        ("abc def", "U+0020"),
    ] {
        let output = run_with_body(&request, &[(CREDENTIAL_NAME, value)], &[])?;
        refused_without_a_decision(&output)?;
        let text = stderr_of(&output);
        if !text.contains(expected) {
            return Err(format!(
                "a credential carrying {expected:?} was refused without saying so: {text:?}"
            ));
        }
    }

    // An oversized credential is refused rather than truncated into a header the provider would
    // reject, and the refusal does not echo it.
    let oversized = "x".repeat(4097);
    let output = run_with_body(&request, &[(CREDENTIAL_NAME, &oversized)], &[])?;
    refused_without_a_decision(&output)?;
    if !stderr_of(&output).contains("longer than this transport will send") {
        return Err(format!(
            "an oversized credential was refused without saying so: {:?}",
            stderr_of(&output)
        ));
    }
    if stderr_of(&output).contains(&oversized) {
        return Err(String::from("a refusal echoed the credential value"));
    }
    Ok(())
}
