// SPDX-License-Identifier: MIT

//! Tests for the restricted-profile dispatch guard.
//!
//! Split from `restricted_dispatch.rs` so the guard and its proof stay independently
//! reviewable: the implementation is a boundary decision, and burying it in 200 lines of
//! test scaffolding is how it stops being read.

use super::super::restricted::ExoToolCatalogError;
use super::{
    ExoToolCatalog, MAX_INSPECTION_DEPTH, MAX_REPORTED_REFUSALS, RefusalReason, RefusedToolCall,
    bare_tool_name, catalog_admitting, refuse_unadmitted_tool_calls,
    refuse_unadmitted_tool_calls_with,
};

/// The seam #127's read-only adapters will use: a catalog whose names are, for the duration of
/// a test, treated as reviewed.
///
/// This is the only way to reach the *admitted* branch of the guard while
/// `REVIEWED_MODEL_TOOLS` is empty. Without it the allow path would be dead and therefore
/// untested, and the day #127 admits its first adapter would land with an unexercised branch.
/// Test-only for that reason: it is not reachable from production code, so it cannot be used to
/// widen an admitted run's authority.
fn reviewed_admitting(tools: &[&str]) -> ExoToolCatalog {
    catalog_admitting(tools)
}

/// Refuses against an explicit reviewed allowlist, so both refusal reasons and the admitted
/// branch are reachable while `REVIEWED_MODEL_TOOLS` is empty.
fn refusals_with(
    catalog: &ExoToolCatalog,
    reviewed: &[&str],
    output: &[u8],
) -> Vec<RefusedToolCall> {
    refuse_unadmitted_tool_calls_with(catalog, reviewed, output)
}

fn empty() -> ExoToolCatalog {
    ExoToolCatalog::reviewed()
}

#[test]
fn the_reviewed_catalog_admits_no_tool_at_all() {
    // The fail-closed default is the whole point: `REVIEWED_MODEL_TOOLS` is empty, so a
    // compliant run emits decisions in the assistant message and never a tool call.
    assert!(empty().tools.is_empty());
    assert!(empty().validate().is_ok());
}

#[test]
fn a_terminal_decision_with_no_tool_call_is_untouched() {
    let output = br#"{"decision":"action","action_id":"play:1","rationale":"take the turn"}"#;
    assert!(refuse_unadmitted_tool_calls(&empty(), output).is_empty());
}

#[test]
fn a_prompt_that_merely_mentions_a_forbidden_tool_is_not_a_call() {
    // Description is not authority: naming `shell` in prose must not trip the guard.
    let output = br#"{"decision":"action","action_id":"play:1",
            "rationale":"I will not use shell or bash; playing the card"}"#;
    assert!(refuse_unadmitted_tool_calls(&empty(), output).is_empty());
}

#[test]
fn a_forbidden_tool_call_is_refused_under_its_bare_name() {
    let output = br#"{"tool":"shell","arguments":{"command":"rm -rf /"}}"#;
    let refusals = refuse_unadmitted_tool_calls(&empty(), output);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(refusals[0].tool, "shell");
    assert_eq!(refusals[0].reason, RefusalReason::Unreviewed);
}

#[test]
fn an_alias_cannot_address_a_different_tool_than_the_catalog_records() {
    // Every spelling below is the same forbidden capability reached a different way.
    for addressed in [
        "shell",
        "functions.shell",
        "mcp__terminal__shell",
        "mcp__filesystem__read_file",
        "functions.run_command",
    ] {
        let output = format!(r#"{{"tool":"{addressed}"}}"#);
        let refusals = refuse_unadmitted_tool_calls(&empty(), output.as_bytes());
        assert_eq!(
            refusals.len(),
            1,
            "{addressed} must be refused, got {refusals:?}"
        );
        assert!(
            !refusals[0].tool.is_empty(),
            "{addressed} must resolve to a bare tool name"
        );
    }
}

#[test]
fn the_openai_function_call_shape_is_inspected() {
    let output = br#"{"type":"function_call","name":"bash",
            "arguments":"{\"command\":\"id\"}"}"#;
    let refusals = refuse_unadmitted_tool_calls(&empty(), output);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(refusals[0].tool, "bash");
}

#[test]
fn a_tool_call_nested_in_a_content_block_is_still_refused() {
    // Deep nesting is exactly where a guard that only reads the top level would miss it.
    let output = br#"{"choices":[{"message":{"content":[
            {"type":"text","text":"calling"},
            {"type":"tool_use","name":"write_file","input":{"path":"/etc/passwd"}}]}}]}"#;
    let refusals = refuse_unadmitted_tool_calls(&empty(), output);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(refusals[0].tool, "write_file");
}

#[test]
fn several_forbidden_calls_are_all_refused_in_document_order() {
    let output = br#"{"tool_calls":[
            {"name":"shell"},{"name":"http_get"},{"name":"read_env"}]}"#;
    let refusals = refuse_unadmitted_tool_calls(&empty(), output);
    let tools: Vec<&str> = refusals.iter().map(|r| r.tool.as_str()).collect();
    assert_eq!(tools, ["shell", "http_get", "read_env"]);
}

#[test]
fn an_admitted_tool_is_not_refused_but_an_unadmitted_reviewed_one_is() {
    // A catalog may carry a reviewed adapter; the guard must then refuse only what that
    // specific run did not admit, and name the difference.
    let catalog = reviewed_admitting(&["live_observation_bootstrap"]);
    let reviewed = ["live_observation_bootstrap"];
    let allowed = br#"{"tool":"live_observation_bootstrap"}"#;
    assert!(
        refusals_with(&catalog, &reviewed, allowed).is_empty(),
        "an admitted tool must not be refused"
    );

    let denied = br#"{"tool":"shell"}"#;
    let refusals = refusals_with(&catalog, &reviewed, denied);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(
        refusals[0].reason,
        RefusalReason::Unreviewed,
        "a name outside the allowlist is unreviewed, whatever this run's catalog holds"
    );
}

#[test]
fn the_reviewed_allowlist_still_gates_what_a_catalog_may_carry() {
    // The seam builds a catalog naming a tool, but that catalog must not become a way to admit
    // an unreviewed tool.
    //
    // Note the two checks answer different questions and this test pins that: `validate` asks
    // "is this catalog admissible at all", and consults `REVIEWED_MODEL_TOOLS`. The dispatch
    // guard asks "did this run admit the name the model addressed", and consults the catalog.
    // The dispatch guard deliberately does *not* re-consult the allowlist — a run's catalog is
    // already required to have passed `validate`, so re-deriving the allowlist here would only
    // duplicate a check that a caller can bypass by skipping validation. `validate` refusing
    // the catalog is therefore the gate, and it does refuse it.
    let catalog = reviewed_admitting(&["live_observation_bootstrap"]);
    assert_eq!(
        catalog.validate(),
        Err(ExoToolCatalogError::UnknownTool),
        "a name outside REVIEWED_MODEL_TOOLS must stay inadmissible"
    );
}

#[test]
fn the_guarded_name_is_the_bare_name_not_the_alias() {
    // An alias must not smuggle a *reviewed* name past a catalog that does not carry it.
    // The alias resolves to the bare name, which this catalog *does* carry, so the guard must
    // admit it. Refusing here would mean the alias form was being checked against a different
    // name than the catalog records — the exact bypass this test exists to close.
    let catalog = reviewed_admitting(&["live_observation_bootstrap"]);
    let reviewed = ["live_observation_bootstrap"];
    let output = br#"{"tool":"functions.live_observation_bootstrap"}"#;
    let refusals = refusals_with(&catalog, &reviewed, output);
    assert!(
        refusals.is_empty(),
        "an alias of an admitted tool must resolve to that tool and be admitted, got {refusals:?}"
    );
}

#[test]
fn a_reviewed_tool_this_run_did_not_admit_is_refused_as_not_admitted() {
    // The other half of the alias case: reviewed by the allowlist, but absent from *this*
    // run's catalog. That must be refused too, and distinguished from `Unreviewed` so an
    // operator can tell "never allowed" from "allowed elsewhere".
    let catalog = reviewed_admitting(&["some_other_reviewed_tool"]);
    let reviewed = ["live_observation_bootstrap", "some_other_reviewed_tool"];
    let output = br#"{"tool":"live_observation_bootstrap"}"#;
    let refusals = refusals_with(&catalog, &reviewed, output);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(refusals[0].tool, "live_observation_bootstrap");
    assert_eq!(refusals[0].reason, RefusalReason::NotAdmitted);
}

#[test]
fn a_non_json_response_is_left_to_the_decision_parser() {
    // Not a refusal this module can justify; `parse_decision` owns it.
    assert!(refuse_unadmitted_tool_calls(&empty(), b"not json at all").is_empty());
}

#[test]
fn a_past_depth_or_count_bound_stops_instead_of_unbounded_walking() {
    // Deeply nested hostile input must not make the boundary check itself unbounded.
    let mut output = String::from(r#"{"tool":"shell"}"#);
    for _ in 0..MAX_INSPECTION_DEPTH + 4 {
        output = format!(r#"{{"a":{output}}}"#);
    }
    let refusals = refuse_unadmitted_tool_calls(&empty(), output.as_bytes());
    assert!(
        refusals.len() <= MAX_REPORTED_REFUSALS,
        "refusal reporting must stay bounded, got {}",
        refusals.len()
    );
}

#[test]
fn alias_resolution_keeps_the_tool_and_drops_the_server() {
    assert_eq!(bare_tool_name("mcp__terminal__shell"), "shell");
    assert_eq!(bare_tool_name("functions.shell"), "shell");
    assert_eq!(bare_tool_name("shell"), "shell");
    assert_eq!(bare_tool_name("mcp__solo"), "solo");
}
