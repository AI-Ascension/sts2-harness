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
//! decision, and the record states that two stages were used.
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
use std::process::{Command, Stdio};
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
    if run(&options).is_err() {
        eprintln!("System One bridge failed validation or transport");
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
        let evidence = record_profile(&bytes, &options.model, gate(options), &mut ask, true)?;
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

/// Runs the operator-owned transport for exactly one bounded exchange.
///
/// Standard input and output are serviced on their own threads, so neither side can deadlock on a
/// full pipe, and the child is killed at the deadline rather than waited on indefinitely.
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
    let writer = std::thread::spawn(move || input.write_all(&payload));
    let reader = std::thread::spawn(move || {
        let mut received = Vec::new();
        output
            .by_ref()
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut received)
            .map(|_| received)
    });
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
    writer.join().map_err(|_| "transport writer failed")??;
    let received = reader.join().map_err(|_| "transport reader failed")??;
    if !status.success() {
        return Err("transport reported failure".into());
    }
    Ok(received)
}

#[cfg(test)]
#[path = "support/sts2_jev_bridge_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "support/jev_tactical_bridge_tests.rs"]
mod tactical_bridge_tests;

#[cfg(test)]
#[path = "support/jev_tactical_fixture.rs"]
mod tactical_fixture;
