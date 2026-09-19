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
    ACTION_QUESTION, DerivedExactFacts, MAX_PRESENTED_OPTIONS, OptionSelection, SYSTEM_ONE_PATH,
    SelectionMode, SystemOneOption, build_described_system_one_request, describe_action,
    ollama_user_content,
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

#[path = "support/selection_framing.rs"]
mod framing_support;
use framing_support::framing;

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
        "confidence_gate": gate(options),
    })
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
    let decision = decide(&bytes, &options.model, gate(options), &mut |body| {
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
    gate: f64,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Box<dyn std::error::Error>> {
    if bytes.len() > LIMIT {
        return Err("request exceeds bound".into());
    }
    let request: Value = serde_json::from_slice(bytes)?;
    let catalog = catalog(&request)?;
    let observation = request
        .get("observation")
        .ok_or("request carries no observation")?;

    // Fold strategically identical entries before asking. Three copies of one card in hand are three
    // catalog entries, and presenting all three splits the probability mass for that play across
    // them, which reads as low confidence in the play rather than a choice between duplicates.
    let selection = OptionSelection::from_observation(observation, MAX_PRESENTED_OPTIONS);
    if selection.mode == SelectionMode::Forced
        && let Some(only) = selection.presented.first()
    {
        // One legal action is not a question. Asking would spend a call to be told the only thing
        // that can happen, so the bridge answers it without a provider exchange.
        return Ok(json!({
            "decision": "action",
            "action_id": only.action_id,
            "rationale": "bridge-authored evidence: one legal action, chosen without a provider call",
            "confidence": 100,
        }));
    }
    let options = present(&selection, observation, &catalog);
    let ids: Vec<String> = options.iter().map(|option| option.id.clone()).collect();

    let state = state_with_derived_facts(&request, observation)?;
    let body = build_described_system_one_request(
        model,
        &state,
        &options,
        &framing(
            request["objective"].as_str().unwrap_or_default(),
            observation,
        ),
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
        &ids,
        gate,
    )?)
}

/// Builds the option set to present, each carrying a description composed from the observation.
///
/// Falls back to the whole catalog when the selection presented nothing usable, so a selection that
/// cannot read this observation costs the run nothing.
fn present(
    selection: &OptionSelection,
    observation: &Value,
    catalog: &[String],
) -> Vec<SystemOneOption> {
    let entries = observation
        .get("legal_actions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let describe = |action_id: &str| -> String {
        entries
            .iter()
            .find(|entry| entry.get("action_id").and_then(Value::as_str) == Some(action_id))
            .map(|entry| describe_action(entry, observation))
            .unwrap_or_else(|| action_id.to_owned())
    };
    if selection.presented.is_empty() {
        return catalog
            .iter()
            .map(|id| SystemOneOption {
                id: id.clone(),
                description: describe(id),
            })
            .collect();
    }
    selection
        .presented
        .iter()
        .map(|option| SystemOneOption {
            id: option.action_id.clone(),
            description: describe(&option.action_id),
        })
        .collect()
}

/// Renders the state, adding the facts that follow exactly from this observation.
///
/// The facts are derived, never fetched: incoming damage is the sum the host's own revealed intents
/// state, and each one is omitted when the observation does not support it exactly. They are added
/// because the arithmetic is the part a System One model is documented not to do, and the state
/// already carries every term of it.
fn state_with_derived_facts(
    request: &Value,
    observation: &Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let rendered = ollama_user_content(request)?;
    let facts = DerivedExactFacts::from_observation(observation);
    let Ok(mut value) = serde_json::from_str::<Value>(&rendered) else {
        return Ok(rendered);
    };
    let Some(object) = value.as_object_mut() else {
        return Ok(rendered);
    };
    object.insert(String::from("derived_exact"), serde_json::to_value(facts)?);
    Ok(value.to_string())
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
