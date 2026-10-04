// SPDX-License-Identifier: MIT

//! Dispatch-time enforcement for the reviewed Exo tool catalog.
//!
//! `ExoToolCatalog::validate` proves a *declaration* is admissible. It says nothing about what a
//! model actually asked for once the answer comes back, so a catalog that validates as empty while
//! the model streams `shell` is, on validation alone, a boundary that has never been crossed by
//! anything. This module is the other half: it reads the model's own output and refuses every tool
//! call the reviewed catalog does not contain.
//!
//! Two properties make the refusal real rather than advisory:
//!
//! * **Dispatch, not description.** The names are read out of the bytes the model returned, so a
//!   prompt that merely *mentions* `shell` is unaffected while a response that *invokes* it is
//!   refused. A tool description cannot grant a capability.
//! * **Alias-aware.** Models address tools by several spellings, so refusal covers the dotted
//!   namespaced form and the conventional MCP prefix as well as the bare name. Aliases are the
//!   obvious way around a bare-name check, so they are checked too.
//!
//! The default catalog is empty, so the fail-closed default here refuses *every* tool call. That is
//! the intended posture until `#127`'s read-only query adapters are admitted: terminal decisions
//! travel in the assistant message, so a compliant run never emits a tool call at all.

use serde_json::Value;

use super::restricted::{ExoToolCatalog, REVIEWED_MODEL_TOOLS};

/// Object keys that may carry a tool call in an OpenAI-style Responses tool-call part. Listed
/// explicitly so a renamed or newly-added upstream field cannot silently stop being inspected.
const TOOL_CALL_KEYS: [&str; 5] = ["name", "tool", "tool_name", "function", "recipient_name"];
/// Keys whose value is itself the tool name, e.g. `{"function": {"name": "shell"}}`.
const NESTED_NAME_KEYS: [&str; 2] = ["name", "tool"];

/// One refused tool call, reported without retaining the model's arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefusedToolCall {
    /// The name as the model addressed it, preserved for the operator-facing refusal.
    pub addressed: String,
    /// The bare tool name the address resolves to.
    pub tool: String,
    /// Why it was refused: not reviewed, or not in the catalog admitted for this run.
    pub reason: RefusalReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalReason {
    /// The name is not in `REVIEWED_MODEL_TOOLS` at all, so no catalog could admit it.
    Unreviewed,
    /// The name is reviewed but this run's catalog does not carry it.
    NotAdmitted,
}

/// Refuses every tool call in `output` that this run's catalog does not admit.
///
/// Returns the refusals in document order. An empty result means the response carried no tool call
/// outside the admitted catalog; it is not a claim that the response was otherwise valid — the
/// decision parser owns that.
#[must_use]
pub fn refuse_unadmitted_tool_calls(
    catalog: &ExoToolCatalog,
    output: &[u8],
) -> Vec<RefusedToolCall> {
    refuse_unadmitted_tool_calls_with(catalog, &REVIEWED_MODEL_TOOLS, output)
}

/// The refusal check against an explicitly supplied reviewed allowlist.
///
/// Production always goes through [`refuse_unadmitted_tool_calls`], which pins the allowlist to
/// [`REVIEWED_MODEL_TOOLS`]. This variant exists so the *admitted* branch is testable at all: while
/// the allowlist is empty a compliant run never emits a tool call, so every reachable outcome is a
/// refusal, and the allow path would otherwise ship unexercised until #127 admits its first adapter.
#[must_use]
fn refuse_unadmitted_tool_calls_with(
    catalog: &ExoToolCatalog,
    reviewed: &[&str],
    output: &[u8],
) -> Vec<RefusedToolCall> {
    let mut refusals = Vec::new();
    inspect(catalog, reviewed, output, 0, &mut refusals);
    refusals
}

/// Depth bound for the walk. Tool calls are shallow; the bound stops a pathological or hostile
/// response from turning a boundary check into unbounded work.
const MAX_INSPECTION_DEPTH: usize = 32;
/// Bound on refusals reported from one response, so a response cannot make refusal itself unbounded.
const MAX_REPORTED_REFUSALS: usize = 64;

fn inspect(
    catalog: &ExoToolCatalog,
    reviewed: &[&str],
    bytes: &[u8],
    depth: usize,
    refusals: &mut Vec<RefusedToolCall>,
) {
    if depth > MAX_INSPECTION_DEPTH || refusals.len() >= MAX_REPORTED_REFUSALS {
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        // Not JSON: `parse_decision` owns the refusal, and a body this module cannot read cannot
        // smuggle a tool call past it, because nothing downstream executes on an unparsed value.
        return;
    };
    walk(catalog, reviewed, &value, depth, refusals);
}

fn walk(
    catalog: &ExoToolCatalog,
    reviewed: &[&str],
    value: &Value,
    depth: usize,
    refusals: &mut Vec<RefusedToolCall>,
) {
    if depth > MAX_INSPECTION_DEPTH || refusals.len() >= MAX_REPORTED_REFUSALS {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                walk(catalog, reviewed, item, depth + 1, refusals);
            }
        }
        Value::Object(object) => {
            let addressed = object
                .iter()
                .filter(|(key, _)| TOOL_CALL_KEYS.contains(&key.as_str()))
                .find_map(|(key, value)| named_tool(key, value).map(|name| (key.clone(), name)));
            if let Some((key, addressed)) = addressed {
                let tool = bare_tool_name(&addressed);
                if !catalog.tools.iter().any(|admitted| admitted == &tool) {
                    refusals.push(RefusedToolCall {
                        reason: if reviewed.contains(&tool.as_str()) {
                            RefusalReason::NotAdmitted
                        } else {
                            RefusalReason::Unreviewed
                        },
                        addressed,
                        tool,
                    });
                }
                // A nested object may hold further tool calls of its own.
                walk(catalog, reviewed, &value[key], depth + 1, refusals);
            }
            for (key, value) in object {
                if !TOOL_CALL_KEYS.contains(&key.as_str()) {
                    walk(catalog, reviewed, value, depth + 1, refusals);
                }
            }
        }
        _ => {}
    }
}

/// Resolves the tool name a key carries, including the `{"function": {"name": ...}}` shape.
fn named_tool(key: &str, value: &Value) -> Option<String> {
    if NESTED_NAME_KEYS.contains(&key)
        && let Some(name) = value.as_str()
    {
        return Some(name.to_owned());
    }
    if key == "function" {
        let object = value.as_object()?;
        for name_key in NESTED_NAME_KEYS {
            if let Some(name) = object.get(name_key).and_then(Value::as_str) {
                return Some(name.to_owned());
            }
        }
    }
    None
}

/// Reduces every spelling a model may use to the bare tool name: a dotted namespace and the
/// conventional `mcp__<server>__<tool>` prefix are both stripped, so an alias cannot address a
/// different name than the catalog records.
#[must_use]
pub fn bare_tool_name(addressed: &str) -> String {
    if let Some(rest) = addressed.strip_prefix("mcp__") {
        // `mcp__<server>__<tool>` — drop the server segment, keep the tool.
        if let Some((_, tool)) = rest.split_once("__") {
            return tool.to_owned();
        }
        return rest.to_owned();
    }
    // The last dotted segment is the tool; `functions.shell` and `shell` are the same tool.
    addressed.rsplit('.').next().unwrap_or(addressed).to_owned()
}

/// Names a catalog that would admit `tool`, for tests and for admitting a future reviewed adapter.
///
/// This does **not** bypass the reviewed allowlist: the returned catalog carries exactly the names
/// given, and `validate` still rejects anything outside [`REVIEWED_MODEL_TOOLS`]. It exists so the
/// dispatch guard's two refusal reasons can both be exercised — with the allowlist empty, only
/// [`RefusalReason::Unreviewed`] is reachable, and a guard whose second branch is dead would be a
/// guard whose second branch is untested.
///
/// Test-only. #127's adapters will admit their own names through the reviewed allowlist, so this
/// helper has no production caller and should not acquire one by accident.
#[cfg(test)]
#[must_use]
pub fn catalog_admitting(tools: &[&str]) -> ExoToolCatalog {
    ExoToolCatalog {
        tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
    }
}

#[cfg(test)]
#[path = "restricted_dispatch_tests.rs"]
mod tests;
