// SPDX-License-Identifier: MIT

//! Synthetic fixtures shared by the final prepared-input budget suites (issues #375 and #381).
//!
//! Every fixture is synthetic: no provider, host, game, or clock is contacted, and every digest is
//! derived from the fixture labels rather than from external content.
#![allow(dead_code)]

use sts2_harness::context_memory::*;

pub fn scope() -> MemoryScope {
    MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
}

pub fn capabilities() -> MemoryCapabilities {
    MemoryCorpus::with_limits(scope(), 16, 4096)
        .expect("corpus")
        .capabilities()
}

pub fn digest(label: &str) -> String {
    sts2_harness::sha256_hex(label.as_bytes())
}

pub fn pins(tokenizer: &str) -> PreparedInputPins {
    PreparedInputPins {
        owner_revision: "harness-context-memory-v3".to_owned(),
        profile_id: "profile-fixture".to_owned(),
        model_id: "model-fixture".to_owned(),
        tokenizer_id: tokenizer.to_owned(),
        adapter_id: "adapter-fixture".to_owned(),
        effective_limit_digest: digest("effective-limits-fixture"),
    }
}

pub fn pinned_entry(entry_id: &str, bytes: usize) -> PinnedInput {
    PinnedInput {
        entry_id: entry_id.to_owned(),
        sha256: digest(&format!("{entry_id}:{bytes}:pinned")),
        content: vec![b'p'; bytes],
    }
}

pub fn optional_entry(entry_id: &str, content: Vec<u8>) -> OptionalInput {
    OptionalInput {
        entry_id: entry_id.to_owned(),
        sha256: digest(&format!("{entry_id}:{}:optional", content.len())),
        content,
    }
}

pub fn base_request() -> PreparedInputRequest {
    PreparedInputRequest {
        request_id: "request-fixture".to_owned(),
        pins: pins("tokenizer-fixture"),
        limits: PreparedInputLimits {
            whole_input_byte_bound: 4096,
            optional_byte_budget: 32,
            output_reserve_bytes: 64,
            combined_window_bytes: None,
        },
        framing: b"frame-v1\n".to_vec(),
        tool_schema: b"{\"type\":\"object\"}".to_vec(),
        mandatory: b"mandatory-fixture".to_vec(),
        pinned: vec![pinned_entry("pin-1", 32)],
        optional: vec![
            optional_entry("opt-1", vec![b'a'; 16]),
            optional_entry("opt-2", vec![b'b'; 16]),
        ],
        measurement: TokenMeasurement::unavailable(MeasurementScope::PreparedInput),
    }
}

pub fn new_ledger() -> MemoryBudgetLedger {
    MemoryBudgetLedger::new(4, 256 * 1024).expect("ledger")
}
