// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The System One bridge observed at its own process boundary.
//!
//! The unit suite inside the binary calls `decide` and `record` in process, so it proves a refusal
//! is *returned* but not that the executable turns that refusal into the exit status and the empty
//! standard output the runtime lane reads. These cases run the real binary against a transport
//! executable, which is the seam the bridge actually owns, and they add the two response classes the
//! in-process suite had no case for: an answer that is not a `200`, and the operator credential.
//!
//! Every refusal case is driven past the request, so the transport is invoked and its marker is
//! written before the refusal happens. A case whose transport never ran would refuse for the wrong
//! reason and would keep passing if the refusal it names were deleted.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// The one line a refused run prints, on standard error rather than standard output.
const FAILURE_LINE: &str = "System One bridge failed validation or transport";

/// The environment name the operator supplies the provider credential in.
const CREDENTIAL_NAME: &str = "TYPESAFE_API_KEY";

/// A value distinctive enough that any appearance of it in an output is a leak.
const CREDENTIAL_VALUE: &str = "typesafe-credential-8f24c1ab-never-recorded";

/// The bridge's own bound on a request, a response and a decision.
const LIMIT: usize = 128 * 1024;

/// A private directory holding one case's transport and the request it received.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Result<Self, String> {
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

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Writes the executable a case hands to `--transport`.
    ///
    /// The body of `reply` runs after the request has been captured and the marker written, so a
    /// case cannot pass by refusing before the provider answer it names ever arrived.
    fn transport(&self, name: &str, reply: &str) -> Result<PathBuf, String> {
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

    fn marker(&self) -> PathBuf {
        self.path("invoked")
    }

    /// Writes the executable a case hands to `--transport`, serving `body` on standard output.
    ///
    /// The body travels in its own file rather than inside the shell script, so no case has to
    /// quote a payload through a shell and a valid answer cannot be mangled into an invalid one.
    fn transport_serving(&self, name: &str, body: &str) -> Result<PathBuf, String> {
        let answer = self.path(&format!("{name}.body"));
        fs::write(&answer, body)
            .map_err(|error| format!("cannot write the provider answer: {error}"))?;
        self.transport(name, &format!("cat '{}'", answer.display()))
    }

    /// The exact bytes the transport received, once a case has run one to completion.
    fn captured_request(&self) -> Result<Vec<u8>, String> {
        fs::read(self.path("request.json"))
            .map_err(|error| format!("the transport received no request: {error}"))
    }

    fn invoked(&self) -> bool {
        self.marker().exists()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A bridge request for a small combat turn, in the shape the runtime lane sends.
fn request() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "model_execution_id": "model-execution-7",
        "objective": "survive the turn",
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

/// A well-formed answer that names one of the presented options.
fn answer(choice: &str, confidence: f64) -> String {
    serde_json::json!({
        "model": "jev-1.13.0",
        "answers": {
            "action": {
                "type": "choice",
                "choice": choice,
                "probabilities": {
                    "play:card-17": 0.62,
                    "play:card-18": 0.21,
                    "combat.end-turn": 0.17,
                },
                "confidence": confidence,
            },
        },
        "usage": {"input_tokens": 900, "output_tokens": 0},
    })
    .to_string()
}

/// Runs the real bridge once, with the request on standard input.
fn run(
    transport: &Path,
    environment: &[(&str, &str)],
    arguments: &[&str],
) -> Result<Output, String> {
    let mut child = launched(transport, environment, arguments)?
        .spawn()
        .map_err(|error| format!("cannot run the bridge: {error}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or("the bridge has no standard input")?;
        stdin
            .write_all(&request())
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
fn run_without_input(
    transport: &Path,
    environment: &[(&str, &str)],
    arguments: &[&str],
) -> Result<Output, String> {
    launched(transport, environment, arguments)?
        .output()
        .map_err(|error| format!("cannot run the bridge: {error}"))
}

fn launched(
    transport: &Path,
    environment: &[(&str, &str)],
    arguments: &[&str],
) -> Result<Command, String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-jev-bridge"));
    command
        .arg("--transport")
        .arg(transport)
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

/// Runs one prepared transport and asserts the refusal came after it ran.
fn assert_refused_after_the_transport_ran(
    scratch: &Scratch,
    name: &str,
    transport: &Path,
) -> Result<(), String> {
    let output = run(transport, &[], &[])?;
    refused_without_a_decision(&output)?;
    if !scratch.invoked() {
        return Err(format!(
            "{name} refused before the provider answer arrived, so the case proves nothing"
        ));
    }
    Ok(())
}

/// A provider answer that is not a `200` is the transport's failure, and the bridge reports it as
/// one: the operator transport exits nonzero and writes nothing.
#[test]
fn a_transport_that_refuses_a_non_200_status_is_refused_without_a_decision() -> Result<(), String> {
    let scratch = Scratch::new("non-200")?;
    let transport = scratch.transport("refuse-503.sh", "exit 22")?;
    assert_refused_after_the_transport_ran(&scratch, "refuse-503.sh", &transport)
}

/// A transport that writes the error *message* instead of the body is refused too, so a status line
/// can never be read as a decision even when the transport claims success.
#[test]
fn a_raw_http_error_status_line_is_refused_without_a_decision() -> Result<(), String> {
    let scratch = Scratch::new("raw-status")?;
    let transport = scratch.transport(
        "raw-503.sh",
        "printf 'HTTP/1.1 503 Service Unavailable\\r\\nContent-Length: 0\\r\\n\\r\\n'",
    )?;
    assert_refused_after_the_transport_ran(&scratch, "raw-503.sh", &transport)
}

#[test]
fn an_out_of_catalog_choice_is_refused_without_a_decision() -> Result<(), String> {
    let scratch = Scratch::new("out-of-catalog")?;
    let transport =
        scratch.transport_serving("out-of-catalog.sh", &answer("play:card-99", 0.99))?;
    assert_refused_after_the_transport_ran(&scratch, "out-of-catalog.sh", &transport)
}

#[test]
fn a_malformed_envelope_is_refused_without_a_decision() -> Result<(), String> {
    let scratch = Scratch::new("malformed")?;
    let transport = scratch.transport_serving("malformed.sh", "{\"answers\": {}}")?;
    assert_refused_after_the_transport_ran(&scratch, "malformed.sh", &transport)
}

/// A well-formed answer that is past the bound is refused as oversized rather than read, and this
/// case can only fail for that reason: the envelope parses and names an option that was presented.
#[test]
fn an_oversized_response_is_refused_without_a_decision() -> Result<(), String> {
    let scratch = Scratch::new("oversized")?;
    let mut oversized: serde_json::Value = serde_json::from_str(&answer("play:card-17", 0.81))
        .map_err(|error| format!("the fixture answer is not an object: {error}"))?;
    oversized["padding"] = serde_json::json!("x".repeat(LIMIT));
    let body = oversized.to_string();
    if body.len() <= LIMIT {
        return Err(format!(
            "the oversized fixture is only {} bytes, so it does not cross the bound",
            body.len()
        ));
    }
    let transport = scratch.transport_serving("oversized.sh", &body)?;
    assert_refused_after_the_transport_ran(&scratch, "oversized.sh", &transport)
}

#[test]
fn a_transport_failure_is_refused_without_a_decision() -> Result<(), String> {
    let scratch = Scratch::new("transport-failure")?;
    let transport = scratch.transport("fail.sh", "exit 3")?;
    assert_refused_after_the_transport_ran(&scratch, "fail.sh", &transport)
}

/// A well-formed answer still produces one decision, so the refusals above are not a bridge that
/// refuses everything.
#[test]
fn a_well_formed_answer_produces_exactly_one_decision() -> Result<(), String> {
    let scratch = Scratch::new("accepted")?;
    let transport = scratch.transport_serving("accept.sh", &answer("play:card-17", 0.81))?;
    let output = run(&transport, &[], &[])?;
    if !output.status.success() {
        return Err(format!("an admitted run failed: {}", stderr_of(&output)));
    }
    let value: serde_json::Value = serde_json::from_str(&stdout_of(&output))
        .map_err(|error| format!("standard output is not one object: {error}"))?;
    if value["decision"] != serde_json::json!("action")
        || value["action_id"] != serde_json::json!("play:card-17")
    {
        return Err(format!("unexpected decision: {value}"));
    }
    Ok(())
}

/// The credential is an environment name the bridge hands to the transport it spawns. It is never
/// an argument, and it never reaches a captured request, a decision, `--describe`, or an error.
#[test]
fn the_credential_never_reaches_a_request_a_decision_or_a_refusal() -> Result<(), String> {
    let credential = [(CREDENTIAL_NAME, CREDENTIAL_VALUE)];

    let scratch = Scratch::new("credential-record")?;
    let transport =
        scratch.transport_serving("credential-record.sh", &answer("play:card-17", 0.81))?;
    let output = run(&transport, &credential, &["--record"])?;
    if !output.status.success() {
        return Err(format!("an admitted run failed: {}", stderr_of(&output)));
    }
    let record = stdout_of(&output);
    let captured = String::from_utf8_lossy(&scratch.captured_request()?).into_owned();
    for (where_, text) in [
        ("the record", record.as_str()),
        ("the request the transport received", captured.as_str()),
        ("the bridge's standard error", stderr_of(&output).as_str()),
    ] {
        if text.contains(CREDENTIAL_VALUE) {
            return Err(format!("the credential value appeared in {where_}"));
        }
    }

    let described = run_without_input(&transport, &credential, &["--describe"])?;
    if !described.status.success() {
        return Err(format!("--describe failed: {}", stderr_of(&described)));
    }
    if stdout_of(&described).contains(CREDENTIAL_VALUE)
        || stderr_of(&described).contains(CREDENTIAL_VALUE)
    {
        return Err(String::from("--describe printed the credential value"));
    }

    // A refusal is text too, and it is the one path that writes a diagnostic.
    let refusal = run(
        &scratch.transport("credential-refusal.sh", "exit 3")?,
        &credential,
        &[],
    )?;
    refused_without_a_decision(&refusal)?;
    if stderr_of(&refusal).contains(CREDENTIAL_VALUE) {
        return Err(String::from("a refusal printed the credential value"));
    }

    // A case asserting only the value's absence would pass even if the name never reached the
    // transport at all, so the positive half is asserted too: the credential arrives by name.
    let dump = scratch.path("environment");
    let probe = scratch.transport("credential-name.sh", &format!("env > '{}'", dump.display()))?;
    let _ = run(&probe, &credential, &[])?;
    let observed = fs::read_to_string(&dump)
        .map_err(|error| format!("the transport did not report its environment: {error}"))?;
    let expected = format!("{CREDENTIAL_NAME}={CREDENTIAL_VALUE}");
    if !observed.lines().any(|line| line == expected) {
        return Err(format!(
            "the declared name did not reach the transport as an environment entry: {observed:?}"
        ));
    }
    Ok(())
}
