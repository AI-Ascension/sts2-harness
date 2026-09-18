// SPDX-License-Identifier: MIT

//! Bridge executable for a System One provider.
//!
//! Reads one bounded decision request on standard input, asks one typed question of the provider,
//! and prints exactly one terminal decision on standard output. Every refusal is fail-closed: a
//! nonzero exit and no decision, never a guessed action.
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
    ACTION_QUESTION, SYSTEM_ONE_PATH, build_system_one_request, ollama_user_content,
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

/// One bounded provider exchange: a request body in, a response body out.
///
/// Named so the offline tests can supply a deterministic fake in place of a network.
type Exchange<'a> = dyn FnMut(&[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> + 'a;

#[path = "support/jev_options.rs"]
mod options;

#[path = "support/systemone_decision.rs"]
mod decision;

fn main() {
    let Ok(options) = options::Options::parse(std::env::args().skip(1)) else {
        eprintln!("Usage: sts2-jev-bridge [--model MODEL] [--transport PATH] [--describe]");
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
    json!({
        "kind": "systemone",
        "provider": PROVIDER,
        "model": options.model,
        "endpoint": format!("https://{PROVIDER_HOST}{SYSTEM_ONE_PATH}"),
        "transport": options.transport,
        "question": ACTION_QUESTION,
        "confidence_gate": decision::DEFAULT_CONFIDENCE_GATE,
    })
}

/// Reads the request, performs one exchange, and prints one decision.
fn run(options: &options::Options) -> Result<(), Box<dyn std::error::Error>> {
    let transport = options
        .transport
        .as_deref()
        .ok_or("a transport executable is required")?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    let decision = decide(&bytes, &options.model, &mut |body| {
        exchange(transport, body, TIMEOUT)
    })?;
    println!("{decision}");
    Ok(())
}

/// Turns one request body into one decision, with the exchange supplied by the caller.
///
/// The exchange is a parameter so the offline tests drive a deterministic fake instead of a network,
/// a credential, and a TLS server.
fn decide(
    bytes: &[u8],
    model: &str,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Box<dyn std::error::Error>> {
    if bytes.len() > LIMIT {
        return Err("request exceeds bound".into());
    }
    let request: Value = serde_json::from_slice(bytes)?;
    let catalog = catalog(&request)?;
    let state = ollama_user_content(&request)?;
    let body = build_system_one_request(
        model,
        &state,
        &catalog,
        request["objective"].as_str().unwrap_or_default(),
        &constraints(&request),
    )?;
    let response = exchange(&serde_json::to_vec(&body)?)?;
    if response.len() > LIMIT {
        return Err("provider response exceeds bound".into());
    }
    let response: Value = serde_json::from_slice(&response)?;
    Ok(decision::map_decision(
        &response,
        ACTION_QUESTION,
        &catalog,
        decision::DEFAULT_CONFIDENCE_GATE,
    )?)
}

/// Reads the host-generated action catalog from the request.
fn catalog(request: &Value) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let ids = request["legal_action_ids"]
        .as_array()
        .ok_or("missing catalog")?;
    if ids.is_empty() || ids.len() > 256 || ids.iter().any(|value| !value.is_string()) {
        return Err("invalid catalog".into());
    }
    Ok(ids
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect())
}

/// Reads the hard constraints from the request, tolerating their absence.
fn constraints(request: &Value) -> Vec<String> {
    request["hard_constraints"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

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
