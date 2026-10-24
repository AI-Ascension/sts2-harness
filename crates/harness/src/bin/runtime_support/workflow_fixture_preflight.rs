// SPDX-License-Identifier: MIT

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sts2_harness::workflow::{
    DecoderLimits, NodeDefinition, WorkflowDefinition, WorkflowMode, decode_strict_with_limits,
    validate_definition,
};

const COMMAND: &str = "validate-r3-fixture";
const ROLE: &str = "workflow_definition";
const SCHEMA: &str = "console18.r3.workflow-fixture-preflight.v1";
const EVIDENCE_CLASS: &str = "source_fixture_validation";
const MAX_INPUT_BYTES: usize = 65_536;
const MAX_INPUT_READ_BYTES: u64 = 65_537;
const MAX_OUTPUT_BYTES: usize = 8_192;
const MAX_DEPTH: usize = 32;
const OUTPUT_FAILURE: &str = concat!(
    "{\"schema\":\"console18.r3.workflow-fixture-preflight.v1\",",
    "\"role\":\"workflow_definition\",\"accepted\":false,",
    "\"evidence_class\":\"source_fixture_validation\",",
    "\"source_sha256\":null,\"refusal_code\":\"output_serialization_failed\"}"
);

#[derive(Serialize)]
struct PreflightResponse {
    schema: &'static str,
    role: &'static str,
    accepted: bool,
    evidence_class: &'static str,
    source_sha256: Option<String>,
    refusal_code: Option<&'static str>,
}

impl PreflightResponse {
    fn refused(code: &'static str) -> Self {
        Self {
            schema: SCHEMA,
            role: ROLE,
            accepted: false,
            evidence_class: EVIDENCE_CLASS,
            source_sha256: None,
            refusal_code: Some(code),
        }
    }

    fn accepted(digest: String) -> Self {
        Self {
            schema: SCHEMA,
            role: ROLE,
            accepted: true,
            evidence_class: EVIDENCE_CLASS,
            source_sha256: Some(digest),
            refusal_code: None,
        }
    }
}

pub(crate) fn run(arguments: &[OsString]) -> i32 {
    if arguments.len() != 2
        || arguments.first().map(OsString::as_os_str) != Some(OsStr::new(COMMAND))
    {
        return respond(PreflightResponse::refused("invalid_arguments"), 2);
    }
    if arguments.get(1).map(OsString::as_os_str) != Some(OsStr::new(ROLE)) {
        return respond(PreflightResponse::refused("unsupported_role"), 2);
    }

    let input = match read_input() {
        Ok(input) => input,
        Err(code) => return respond(PreflightResponse::refused(code), 2),
    };
    if input.len() > MAX_INPUT_BYTES {
        return respond(PreflightResponse::refused("input_too_large"), 2);
    }

    let limits = DecoderLimits {
        max_bytes: MAX_INPUT_BYTES,
        max_depth: MAX_DEPTH,
        ..DecoderLimits::default()
    };
    let definition = match decode_strict_with_limits::<WorkflowDefinition>(&input, limits) {
        Ok(definition) => definition,
        Err(_) => return respond(PreflightResponse::refused("invalid_workflow_json"), 2),
    };
    if validate_definition(&definition).is_err() || !is_r3_workflow(&definition) {
        return respond(PreflightResponse::refused("invalid_workflow_definition"), 2);
    }

    let digest = Sha256::digest(&input);
    let digest = sts2_harness::hex_bytes(&digest[..]);
    respond(PreflightResponse::accepted(digest), 0)
}

fn read_input() -> Result<Vec<u8>, &'static str> {
    let mut input = Vec::with_capacity(MAX_INPUT_BYTES + 1);
    io::stdin()
        .lock()
        .take(MAX_INPUT_READ_BYTES)
        .read_to_end(&mut input)
        .map_err(|_| "input_read_failed")?;
    Ok(input)
}

fn is_r3_workflow(definition: &WorkflowDefinition) -> bool {
    if definition.mode != WorkflowMode::Strict || definition.game_profile.as_str() != "sts2-live-v1"
    {
        return false;
    }

    let mut decision_count = 0usize;
    let mut expected_decision = false;
    for node in definition.graphs.iter().flat_map(|graph| &graph.nodes) {
        if let NodeDefinition::Decide { id, config } = node {
            decision_count = decision_count.saturating_add(1);
            expected_decision = id.as_str() == "decide"
                && config.decision_profile_ref.as_str() == "decision.live.v1"
                && config.context_ref.as_str() == "context.live.v1";
        }
    }
    decision_count == 1 && expected_decision
}

fn respond(response: PreflightResponse, exit_code: i32) -> i32 {
    let (mut output, exit_code) = match serde_json::to_vec(&response) {
        Ok(output) if output.len() < MAX_OUTPUT_BYTES => (output, exit_code),
        _ => (OUTPUT_FAILURE.as_bytes().to_vec(), 2),
    };
    output.push(b'\n');
    if io::stdout().lock().write_all(&output).is_ok() {
        exit_code
    } else {
        2
    }
}
