// SPDX-License-Identifier: MIT

use std::io::{self, Read};
use sts2_harness::context_memory::*;

const NOW: &str = "2026-09-10T12:00:00Z";
const LATER: &str = "2026-09-11T12:00:00Z";

fn scope() -> MemoryScope {
    MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
}

fn corpus() -> Result<MemoryCorpus, MemoryError> {
    let mut corpus = MemoryCorpus::with_limits(scope(), 16, 16 * 1024)?;
    corpus.admit(MemoryEntry::new(
        scope(),
        "hist-1",
        "record-hist-1",
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch-a",
        "hist-1",
        b"HP loss was 2 and the action settled".to_vec(),
        4,
        5,
        5,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    ))?;
    corpus.admit(MemoryEntry::new(
        scope(),
        "hist-2",
        "record-hist-2",
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Reported,
        "branch-a",
        "hist-2",
        b"Potion route was selected".to_vec(),
        5,
        6,
        5,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    ))?;
    Ok(corpus)
}

fn open_corpus() -> Result<MemoryCorpus, String> {
    corpus().map_err(|error| error.to_string())
}

fn read_stdin() -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|error| format!("stdin: {error}"))?;
    if bytes.len() > MAX_JOB_INPUT_BYTES {
        return Err("input exceeds bounded memory CLI limit".to_owned());
    }
    Ok(bytes)
}

fn print_json(value: impl serde::Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(&value).map_err(|error| format!("encode: {error}"))?;
    println!(
        "{}",
        String::from_utf8(bytes).map_err(|_| "response is not UTF-8".to_owned())?
    );
    Ok(())
}

fn policy(generation: u64) -> MemoryPolicy {
    MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "policy-cli".to_owned(),
        version: 1,
        scope: scope(),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("revision-cli".to_owned()),
        corpus_generation: generation,
        rolling_same_episode_sources: false,
        cross_scope: false,
        approved_summary_catalog: Vec::new(),
        ranker_version: "lexical-v1".to_owned(),
        query_derivation_version: "derive-v1".to_owned(),
        max_candidates: 64,
        max_results: 16,
        max_selected: 16,
        optional_byte_budget: 8192,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    }
}

fn main() {
    let operation = std::env::args().nth(1).unwrap_or_else(|| "help".to_owned());
    let result = match operation.as_str() {
        "help" | "--help" | "-h" => {
            println!("context-memory-cli capabilities|status|list|search|extract|policy-preview");
            println!("  search/extract/policy-preview read strict JSON from stdin");
            Ok(())
        }
        "capabilities" => open_corpus().and_then(|corpus| print_json(corpus.capabilities())),
        "status" => open_corpus().and_then(|corpus| {
            print_json(serde_json::json!({
                "schema": "ascension.context-memory.status.v1",
                "enabled": corpus.enabled(),
                "corpus_generation": corpus.generation(),
                "projection_generation": corpus.projection_generation(),
                "revocation_epoch": corpus.revocation_epoch(),
                "inference_calls": 0,
                "effect_class": "local_read_no_inference"
            }))
        }),
        "list" => open_corpus().and_then(|corpus| print_json(corpus.entries().collect::<Vec<_>>())),
        "search" => open_corpus().and_then(|corpus| {
            let query: MemoryQuery = parse_strict_json(&read_stdin()?)
                .map_err(|_| "invalid query.v1 document".to_owned())?;
            let response = corpus
                .retrieve(&query, NOW)
                .map_err(|error| error.to_string())?;
            print_json(response)
        }),
        "extract" => open_corpus().and_then(|corpus| {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Request {
                source_ids: Vec<String>,
                branch_id: String,
                cutoff: u64,
                corpus_generation: u64,
            }
            let request: Request = parse_strict_json(&read_stdin()?)
                .map_err(|_| "invalid extract request".to_owned())?;
            let refs = request
                .source_ids
                .iter()
                .map(|id| {
                    corpus
                        .entries()
                        .find(|entry| entry.entry_id == *id)
                        .map(MemoryEntry::reference)
                        .ok_or_else(|| "source is unavailable".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let proposal = corpus
                .exact_extract(
                    "proposal-cli",
                    &refs,
                    &request.branch_id,
                    request.cutoff,
                    request.corpus_generation,
                    NOW,
                    NOW,
                    LATER,
                )
                .map_err(|error| error.to_string())?;
            print_json(proposal)
        }),
        "policy-preview" => open_corpus().and_then(|corpus| {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Request {
                query: MemoryQuery,
                mandatory_bytes: Vec<u8>,
                phase2_prepared_manifest_sha256: String,
                expires_at: String,
            }
            let request: Request = parse_strict_json(&read_stdin()?)
                .map_err(|_| "invalid policy preview request".to_owned())?;
            let realized = realize_policy(
                &corpus,
                &policy(corpus.generation()),
                "selection-cli",
                &request.query,
                request.mandatory_bytes,
                request.phase2_prepared_manifest_sha256,
                "prepared-cli",
                request.expires_at,
                NOW,
            )
            .map_err(|error| error.to_string())?;
            print_json(realized.selection)
        }),
        _ => Err("unsupported context-memory-cli operation".to_owned()),
    };
    if let Err(error) = result {
        eprintln!("context-memory-cli: {error}");
        std::process::exit(2);
    }
}
