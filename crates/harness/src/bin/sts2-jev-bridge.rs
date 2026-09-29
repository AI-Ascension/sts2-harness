// SPDX-License-Identifier: MIT

//! Bridge executable for a System One provider.
//!
//! Reads one bounded decision request on standard input, asks one typed question of the provider,
//! and prints exactly one terminal decision on standard output. Every refusal is fail-closed: a
//! nonzero exit and no decision, never a guessed action.
//!
//! When the presented option set is larger than the question bound, the ask is split in two: a
//! `kind` question followed by an `action` question restricted to the chosen kind. That keeps each
//! question small, which is the reason the bound exists; both stages still print exactly one terminal
//! decision, and the record states that two stages were used. The split needs two exchanges, so the
//! profiles that permit only one transport invocation — the capture profile (`--audit-dir`) and the
//! evaluation profile (`--tactical`) — keep asking the single question they were reviewed with.
//!
//! `--record` prints one object carrying the provider request, the provider response, and the
//! decision, instead of the decision alone. The decision in that record is composed from the
//! response in the same record, so an evidence file built from it can show the decision is a
//! function of the response rather than a second field written down separately. The default output
//! is unchanged.
//!
//! The HTTPS exchange itself is performed by an operator-owned transport executable, named by
//! `--transport` and pinned by the operator's own digest, following the precedent
//! `sts2-astra-bridge` set for a provider whose transport this repository does not own. This binary
//! keeps everything that has to stay reviewable: request construction, bounds, catalog membership,
//! the confidence gate, and the decision shape. See ADR 0053.
//!
//! The transport contract is deliberately tiny. The transport reads one request body on standard
//! input, performs one `POST` to the provider endpoint with the bearer credential from its own
//! environment, and writes the response body — the JSON, not an HTTP message — on standard output.
//! It never sees the catalog or the decision, and the credential never reaches this process.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use sts2_harness::{
    ACTION_QUESTION, KIND_QUESTION, MAX_PRESENTED_OPTIONS, OptionSelection, SYSTEM_ONE_PATH,
    SelectionMode, build_class_system_one_request, build_described_system_one_request,
};

/// Largest request, response, and decision this bridge handles.
const LIMIT: usize = 128 * 1024;

/// How long the transport has to complete one exchange.
const TIMEOUT: Duration = Duration::from_secs(100);

/// How often the transport is checked for completion.
const POLL: Duration = Duration::from_millis(25);

/// Provider host this lane is admitted against.
const PROVIDER_HOST: &str = "api.typesafe.ai";

/// Provider identity recorded beside a run.
const PROVIDER: &str = "typesafe";

/// Schema identifier carried by one bridge record.
///
/// A record is the bridge's own statement of what it asked, what came back, and what it decided;
/// an evidence file that publishes a decision beside a response can carry this record instead of
/// two fields transcribed by hand.
const RECORD_SCHEMA: &str = "ascension.system-one-bridge-record.v1";

/// One bounded provider exchange: a request body in, a response body out.
///
/// Named so the offline tests can supply a deterministic fake in place of a network.
type Exchange<'a> = dyn FnMut(&[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> + 'a;

#[path = "support/jev_options.rs"]
mod options;

#[path = "support/jev_tactical.rs"]
mod tactical;

#[path = "support/jev_capture.rs"]
mod capture;

#[path = "support/systemone_decision.rs"]
mod decision;

#[path = "support/selection_framing.rs"]
mod framing_support;
use framing_support::framing;

#[path = "support/system_one_request.rs"]
mod request_support;
use request_support::{catalog, constraints, present, state_with_derived_facts};

fn main() {
    let Ok(options) = options::Options::parse(std::env::args().skip(1)) else {
        eprintln!(
            "Usage: sts2-jev-bridge [--model MODEL] [--transport PATH] [--gate PERCENT] \
             [--record] [--describe] [--tactical] [--audit-dir DIR]"
        );
        std::process::exit(2);
    };
    if options.describe {
        println!("{}", describe(&options));
        return;
    }
    if let Err(error) = run(&options) {
        eprintln!("System One bridge failed validation or transport");
        // A launch that never happened and a behavioural refusal are the same exit status and the
        // same first line, so a reader of this process cannot tell them apart. The test suite
        // asserts only the first line, so this second line is additive: it names the cause without
        // changing what a refused run prints as its contract.
        eprintln!("cause: {error}");
        std::process::exit(2);
    }
}

/// Reports the requested configuration without opening a connection or reading input.
///
/// This is requested configuration, not availability, and not evidence that inference happened.
fn describe(options: &options::Options) -> Value {
    let mut description = json!({
        "kind": "systemone",
        "provider": PROVIDER,
        "model": options.model,
        "endpoint": format!("https://{PROVIDER_HOST}{SYSTEM_ONE_PATH}"),
        "transport": options.transport,
        "question": ACTION_QUESTION,
        "confidence_gate": gate(options),
    });
    if options.tactical {
        description["tactical_profile"] = json!(tactical::PROFILE);
    }
    if options.audit_dir.is_some() {
        description["redacted_capture"] = json!(capture::SCHEMA);
    }
    description
}

/// The confidence gate this invocation applies.
///
/// An operator-supplied percentage wins over the bridge's default, which is a starting value rather
/// than a calibrated one.
fn gate(options: &options::Options) -> f64 {
    options
        .gate_percent
        .map_or(decision::DEFAULT_CONFIDENCE_GATE, |percent| {
            f64::from(percent) / 100.0
        })
}

/// Reads the request, performs one exchange, and prints one decision or one record.
fn run(options: &options::Options) -> Result<(), Box<dyn std::error::Error>> {
    let transport = options
        .transport
        .as_deref()
        .ok_or("a transport executable is required")?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    let mut ask = |body: &[u8]| exchange(transport, body, TIMEOUT);
    if options.audit_dir.is_some() {
        println!("{}", capture::run(&bytes, options, &mut ask)?);
    } else if options.tactical {
        let evidence =
            record_profile(&bytes, &options.model, gate(options), &mut ask, true, false)?;
        let output = if options.record {
            evidence
        } else {
            evidence["decision"].clone()
        };
        println!("{output}");
    } else if options.record {
        println!(
            "{}",
            record(&bytes, &options.model, gate(options), &mut ask)?
        );
    } else {
        // The default output stays exactly one decision object, so an admitted invocation that did
        // not ask for a record reads the same bytes it read before.
        println!(
            "{}",
            decide(&bytes, &options.model, gate(options), &mut ask)?
        );
    }
    Ok(())
}

/// Turns one request body into one decision, with the exchange supplied by the caller.
///
/// The exchange is a parameter so the offline tests drive a deterministic fake instead of a network,
/// a credential, and a TLS server.
fn decide(
    bytes: &[u8],
    model: &str,
    gate: f64,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(record(bytes, model, gate, exchange)?["decision"].clone())
}

/// Turns one request body into one record carrying what was asked, what came back, and the decision.
///
/// The record exists so the decision can be shown to be a function of the response it is stored
/// beside. `provider_call` is `false` when the bridge answered without asking, and the two provider
/// fields are then `null` rather than a fabricated exchange.
#[path = "support/jev_record.rs"]
mod recording;
use recording::{record, record_profile};

#[path = "support/jev_transport_worker.rs"]
mod transport_worker;
use transport_worker::{Reader, Writer, join_writer, spawn_transport_worker};

/// Runs the operator-owned transport for exactly one bounded exchange.
///
/// Standard input and output are serviced on their own threads, so neither side can deadlock on a
/// full pipe, and the child is killed at the deadline rather than waited on indefinitely.
///
/// A thread that cannot be created is a transport failure, not a reason to abort the process. The
/// two workers are therefore built with `Builder::spawn`, which reports `EAGAIN` as an `Err`
/// instead of panicking the way `std::thread::spawn` does. On a contended host the panic would
/// unwind through `main`, so the process would die with a test-harness abort rather than the
/// refusal's own exit status, and the caller would see a transport that never ran.
///
/// The credential is never passed here: the transport reads it from the environment the runtime
/// declared, so it is never an argument of this process, a captured byte, or a record.
fn exchange(
    transport: &str,
    body: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut child = Command::new(transport)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("transport stdin is unavailable")?;
    let mut output = child
        .stdout
        .take()
        .ok_or("transport stdout is unavailable")?;
    let payload = body.to_vec();
    // `std::thread::spawn` panics when the host cannot create a thread (`EAGAIN` on process
    // slots), and a panic here takes the whole process down with exit 101 -- indistinguishable
    // from a crash, and with no `Err` for `main` to report. `Builder::spawn` returns that failure
    // instead, so a host that cannot fork is reported as the transport failure it is.
    //
    // Both arms kill the child on the way out, because `Child`'s `Drop` closes its handles without
    // signalling the process. The writer arm needs it as much as the reader arm: the transport is
    // already running by the time the first thread is attempted, so a failure there would orphan
    // it just as surely on the host that had no spare thread to give.
    let writer = spawn_transport_worker(
        std::thread::Builder::new().name("jev-transport-writer".to_owned()),
        Writer,
        move || input.write_all(&payload),
    )
    .map_err(|error| {
        kill_child(
            &mut child,
            format!("cannot start the transport writer: {error}"),
        )
    })?;
    let reader = spawn_transport_worker(
        std::thread::Builder::new().name("jev-transport-reader".to_owned()),
        Reader,
        move || {
            let mut received = Vec::new();
            output
                .by_ref()
                .take((LIMIT + 1) as u64)
                .read_to_end(&mut received)
                .map(|_| received)
        },
    )
    // The reader is the second thread, so it is the one that can fail after the writer is
    // already running. Returning here would drop the writer's handle and leave the child
    // waiting on a pipe nobody is servicing, so the child is killed on the way out.
    .map_err(|error| {
        kill_child(
            &mut child,
            format!("cannot start the transport reader: {error}"),
        )
    })?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err("transport timeout".into());
        }
        std::thread::sleep(POLL);
    };
    // The status is read BEFORE either worker is joined, because it does not depend on scheduling
    // and the writer's `EPIPE` usually beats it (Refs #751); `join_writer` absorbs that same early
    // close afterwards (Refs #753).
    if !status.success() {
        return Err("transport reported failure".into());
    }
    join_writer(writer)?;
    let received = reader.join().map_err(|_| "transport reader failed")??;
    Ok(received)
}

/// Kills `child`, then hands back the error that made the bridge give up on it.
///
/// `std::process::Child` has no `Drop` that signals the process -- dropping it closes the handles
/// and leaves the transport running -- so returning a spawn failure without this would report the
/// failure correctly and still orphan a child, on precisely the host that had nothing to spare.
fn kill_child(child: &mut Child, error: String) -> String {
    let _ = child.kill();
    error
}
#[cfg(test)]
#[path = "support/sts2_jev_bridge_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "support/jev_transport_child_tests.rs"]
mod transport_child_tests;

#[cfg(test)]
#[path = "support/jev_tactical_bridge_tests.rs"]
mod tactical_bridge_tests;

#[cfg(test)]
#[path = "support/jev_tactical_fixture.rs"]
mod tactical_fixture;
