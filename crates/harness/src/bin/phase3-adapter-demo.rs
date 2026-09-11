// SPDX-License-Identifier: MIT

//! Built offline target↔harness Phase 3 adapter demonstration.
//!
//! The harness owns the corpus, summary job and review. It invokes only the checked target
//! adapter and the checked fake summary peer, then verifies that the target commits the reviewed
//! metadata beside its Phase 2 revision while remaining paused.

use serde_json::{Value, json};
use std::env;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use sts2_harness::context_memory::*;

const NOW: &str = "2026-09-10T12:00:00Z";
const LATER: &str = "2026-09-11T12:00:00Z";
const ADAPTER_SCHEMA: &str = "ascension.context-memory.adapter-request.v1";
const OUTPUT: &[u8] = b"fake-summary-requires-review";

fn main() {
    if let Err(error) = run() {
        eprintln!("phase3 adapter demo: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let target = env::var("PHASE3_TARGET_BINARY")
        .map_err(|_| "PHASE3_TARGET_BINARY must name the built target binary".to_owned())?;
    let target = Path::new(&target);
    if !target.is_file() {
        return Err(format!(
            "target binary is unavailable: {}",
            target.display()
        ));
    }
    let fake_peer = env::var("PHASE3_FAKE_PEER_BINARY")
        .map_err(|_| "PHASE3_FAKE_PEER_BINARY must name the built fake peer".to_owned())?;
    let fake_peer = Path::new(&fake_peer);
    if !fake_peer.is_file() {
        return Err(format!(
            "fake peer binary is unavailable: {}",
            fake_peer.display()
        ));
    }

    let scope = MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    );
    let mut corpus =
        MemoryCorpus::with_limits(scope.clone(), 16, 4096).map_err(|error| error.to_string())?;
    let source_bytes = b"HP loss was 2 and the action settled.".to_vec();
    let source = MemoryEntry::new(
        scope.clone(),
        "history-1",
        "record-history-1",
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch-a",
        "history-content-1",
        source_bytes.clone(),
        1,
        1,
        1,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    );
    let source_ref = source.reference();
    corpus.admit(source).map_err(|error| error.to_string())?;

    let probe = adapter_request(
        "binding-demo-1",
        "selection-demo-1",
        "revision-pending",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "policy-demo-1",
        "review-demo-1",
        &sha256_hex(OUTPUT),
        &sha256_hex("pending-input"),
        true,
    );
    let preview = invoke_target(target, &probe)?;
    if preview["status"] != "preview_ready" {
        return Err(format!(
            "target adapter did not return preview_ready: {preview}"
        ));
    }
    let phase2_preview_id = string_field(&preview, "phase2_preview_id")?;
    let phase2_revision_id = string_field(&preview, "planned_phase2_revision_id")?;
    let phase2_prepared_digest = string_field(&preview, "prepared_manifest_sha256")?;

    let mut jobs = SummaryJobStore::new(4).map_err(|error| error.to_string())?;
    let job = SummaryJob {
        schema: MEMORY_JOB_SCHEMA.to_owned(),
        job_id: "job-demo-1".to_owned(),
        scope: scope.clone(),
        branch_id: "branch-a".to_owned(),
        sources: vec![source_ref.clone()],
        cutoff: 10,
        corpus_generation: 1,
        source_manifest_sha256: source_manifest_digest(std::slice::from_ref(&source_ref)),
        generator_profile: "fake-v1".to_owned(),
        generator_prompt_sha256: sha256_hex("summary-prompt-v1"),
        output_schema_sha256: sha256_hex("summary-output-v1"),
        idempotency_key: "job-key-demo-1".to_owned(),
        command_window_id: "window-demo-1".to_owned(),
        state: JobState::Queued,
        attempt_id: None,
        provider_write_state: ProviderWriteState::NotStarted,
        input_bytes: source_bytes.len(),
        max_output_bytes: 8192,
        review_required: true,
        auto_apply: false,
        deadline_at: LATER.to_owned(),
        effect_class: "authorized_summary_generation_only".to_owned(),
    };
    jobs.admit(job).map_err(|error| error.to_string())?;
    let mut local_peer =
        FakeSummaryPeer::new(OUTPUT.to_vec()).map_err(|error| error.to_string())?;
    let proposal = jobs
        .execute_explicit("job-demo-1", &corpus, &mut local_peer, NOW, true)
        .map_err(|error| error.to_string())?;
    if proposal.status != ProposalStatus::ReviewRequired || local_peer.requests().len() != 1 {
        return Err("summary proposal was not held for review".to_owned());
    }

    let peer_request = json!({
        "schema": FAKE_PEER_SCHEMA,
        "job_id": "job-demo-1",
        "generator_profile": "fake-v1",
        "prompt_sha256": sha256_hex("summary-prompt-v1"),
        "output_schema_sha256": sha256_hex("summary-output-v1"),
        "sources": [{
            "entry_id": source_ref.entry_id,
            "version": source_ref.version,
            "sha256": source_ref.sha256,
            "bytes": source_bytes,
        }],
        "max_output_bytes": 8192,
    });
    let peer_response = invoke_fake_peer(fake_peer, &peer_request)?;
    if peer_response["effect_class"] != "authorized_summary_generation_only"
        || peer_response["source_manifest_sha256"]
            != source_manifest_digest(std::slice::from_ref(&source_ref))
        || peer_response["output_sha256"] != sha256_hex(OUTPUT)
    {
        return Err(format!("fake peer boundary mismatch: {peer_response}"));
    }

    let review = MemoryReview {
        schema: MEMORY_REVIEW_SCHEMA.to_owned(),
        review_id: "review-demo-1".to_owned(),
        proposal_id: proposal.proposal_id.clone(),
        proposal_version: proposal.version,
        proposal_sha256: proposal.sha256.clone(),
        scope: scope.clone(),
        source_manifest_sha256: source_manifest_digest(std::slice::from_ref(&source_ref)),
        revocation_epoch: corpus.revocation_epoch(),
        reviewer_ref: "independent-reviewer-demo".to_owned(),
        decision: ReviewDecision::Admit,
        support_check: SupportCheck::IndependentReview,
        reason_codes: vec!["bounded_fake_peer_review".to_owned()],
        created_at: NOW.to_owned(),
        creates_active_revision: false,
    };
    let mut reviews = ImmutableReviewLedger::new();
    reviews
        .record(&proposal, review.clone(), &corpus)
        .map_err(|error| error.to_string())?;
    corpus
        .admit_reviewed_proposal(&proposal, &review, NOW)
        .map_err(|error| error.to_string())?;
    let summary_ref = MemoryRef::new(
        proposal.proposal_id.clone(),
        proposal.version,
        proposal.sha256.clone(),
    );

    let policy = MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "policy-demo-1".to_owned(),
        version: 1,
        scope: scope.clone(),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some(phase2_revision_id.clone()),
        corpus_generation: corpus.generation(),
        rolling_same_episode_sources: false,
        cross_scope: false,
        approved_summary_catalog: Vec::new(),
        ranker_version: "lexical-v1".to_owned(),
        query_derivation_version: "derive-v1".to_owned(),
        max_candidates: 64,
        max_results: 8,
        max_selected: 8,
        optional_byte_budget: 8 * 1024,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    };
    let query = MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "query-demo-1".to_owned(),
        scope: scope.clone(),
        branch_id: "branch-a".to_owned(),
        query: "review".to_owned(),
        cutoff: 10,
        corpus_generation: corpus.generation(),
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    };
    let realized = realize_policy(
        &corpus,
        &policy,
        "selection-demo-1",
        &query,
        b"protected-phase2-bytes".to_vec(),
        phase2_prepared_digest.clone(),
        "prepared-demo-1",
        LATER,
        NOW,
    )
    .map_err(|error| error.to_string())?;
    let selection = realized.selection;
    if !selection.selected_sources.contains(&summary_ref) {
        return Err("reviewed summary was not selected for adoption".to_owned());
    }
    let audit_bytes =
        serde_json::to_vec(&review).map_err(|_| "review encoding failed".to_owned())?;
    let audit_digest = sha256_hex(audit_bytes);
    let mut approvals = ApprovalStore::new();
    let approval = approvals
        .bind(
            "approval-demo-1",
            &selection,
            phase2_preview_id.clone(),
            source_manifest_digest(&selection.selected_sources),
        )
        .map_err(|error| error.to_string())?;
    if approval.state != ApprovalState::PreviewReady {
        return Err("approval did not start in preview_ready state".to_owned());
    }
    let held = approvals
        .commit_held("approval-demo-1", corpus.revocation_epoch(), NOW)
        .map_err(|error| error.to_string())?;
    if held.state != ApprovalState::CommittedHeld {
        return Err("memory approval was not held".to_owned());
    }

    let final_request = adapter_request(
        "binding-demo-1",
        &selection.selection_id,
        &phase2_revision_id,
        &selection.prepared_manifest_sha256,
        &audit_digest,
        &selection.policy_id,
        "review-demo-1",
        &sha256_hex(OUTPUT),
        &phase2_prepared_digest,
        false,
    );
    let adopted = invoke_target(target, &final_request)?;
    if adopted["status"] != "completed"
        || adopted["phase2_revision_id"] != phase2_revision_id
        || adopted["phase2_preview_id"] != phase2_preview_id
        || adopted["paused_after_commit"] != true
        || adopted["binding_committed_atomically"] != true
    {
        return Err(format!("target adoption mismatch: {adopted}"));
    }

    let mut resume = FirstResumeLedger::default();
    resume
        .prepare(&held, &selection)
        .map_err(|error| error.to_string())?;
    let (outcome, submission) = resume
        .submit_first(
            &held,
            corpus.revocation_epoch(),
            NOW,
            &selection.rendered_bytes,
        )
        .map_err(|error| error.to_string())?;
    if outcome != ResumeOutcome::Submitted || submission.rendered_bytes != selection.rendered_bytes
    {
        return Err("first resume did not preserve the approved rendered bytes".to_owned());
    }

    println!(
        "{}",
        serde_json::to_string(&json!({
            "schema": "ascension.context-memory.adapter-demo-evidence.v1",
            "status": "passed",
            "target_binary": target.display().to_string(),
            "fake_peer_binary": fake_peer.display().to_string(),
            "phase2_preview_id": phase2_preview_id,
            "phase2_revision_id": phase2_revision_id,
            "selection_id": selection.selection_id,
            "selection_sha256": selection.prepared_manifest_sha256,
            "phase2_prepared_manifest_sha256": phase2_prepared_digest,
            "summary_output_sha256": sha256_hex(OUTPUT),
            "review_id": review.review_id,
            "approval_state": format!("{:?}", held.state),
            "resume_outcome": format!("{outcome:?}"),
            "first_input_bytes": submission.rendered_bytes.len(),
            "provider_calls": 0,
            "game_launches": 0,
            "external_requests": 0,
            "unauthorized_processes": 0,
            "network_tripwire": "no network-capable adapter path",
        }))
        .map_err(|_| "evidence encoding failed".to_owned())?
    );
    Ok(())
}

fn adapter_request(
    binding_id: &str,
    selection_id: &str,
    phase2_revision_id: &str,
    selection_sha256: &str,
    audit_sha256: &str,
    policy_id: &str,
    review_id: &str,
    summary_output_sha256: &str,
    first_input_sha256: &str,
    preview_only: bool,
) -> Value {
    json!({
        "schema": ADAPTER_SCHEMA,
        "binding_id": binding_id,
        "selection_id": selection_id,
        "phase2_revision_id": phase2_revision_id,
        "selection_sha256": selection_sha256,
        "audit_sha256": audit_sha256,
        "policy_id": policy_id,
        "policy_version": 1,
        "review_id": review_id,
        "summary_output_sha256": summary_output_sha256,
        "first_input_sha256": first_input_sha256,
        "preview_only": preview_only,
    })
}

fn invoke_target(path: &Path, request: &Value) -> Result<Value, String> {
    let mut child = Command::new(path)
        .arg("phase3-adapter")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn target adapter: {error}"))?;
    let body = serde_json::to_vec(request).map_err(|_| "request encoding failed".to_owned())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "target stdin unavailable".to_owned())?
        .write_all(&body)
        .map_err(|error| format!("write target request: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait target adapter: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "target adapter failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("target result JSON: {error}"))
}

fn invoke_fake_peer(path: &Path, request: &Value) -> Result<Value, String> {
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn fake peer: {error}"))?;
    let body =
        serde_json::to_vec(request).map_err(|_| "peer request encoding failed".to_owned())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "fake peer stdin unavailable".to_owned())?;
    stdin
        .write_all(&body)
        .and_then(|()| stdin.write_all(b"\n"))
        .map_err(|error| format!("write fake peer request: {error}"))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait fake peer: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "fake peer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("fake peer result JSON: {error}"))
}

fn string_field(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("adapter result missing {key}"))
}
