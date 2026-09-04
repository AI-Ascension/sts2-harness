// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

mod digests;
mod fixtures;
mod integrity;
#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod tests;

const EXPECTED_STATES: usize = 131;
const EXPECTED_OBSERVATIONS: usize = 4_315;
const EXPECTED_ACTIONS: usize = 421;
const EXPECTED_TRANSITIONS: usize = 1_059;
const GENERATOR_VERSION: &str = "sts2-expert-state-package-generator/1";

fn main() -> Result<(), Box<dyn Error>> {
    let root = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docs/research/sts2-expert-state-package"));
    let records = load_states(&root)?;
    validate_state_ids(&records)?;
    validate_record_count(&root, "observations.json", EXPECTED_OBSERVATIONS)?;
    validate_record_count(&root, "actions.json", EXPECTED_ACTIONS)?;
    validate_record_count(&root, "transitions.json", EXPECTED_TRANSITIONS)?;
    integrity::validate(&root, &records)?;

    let report_dir = root.join("report");
    let diagram_dir = root.join("diagrams").join("states");
    fs::create_dir_all(&report_dir)?;
    fs::create_dir_all(&diagram_dir)?;

    for record in records {
        let state_id = required_string(&record, "state_id")?;
        let slug = safe_slug(state_id);
        fs::write(
            report_dir.join(format!("{slug}.md")),
            render_state_report(&record)?,
        )?;
        fs::write(
            diagram_dir.join(format!("{slug}.mmd")),
            render_state_diagram(&record)?,
        )?;
    }

    fs::write(
        root.join("diagrams/global-architecture.mmd"),
        global_architecture(),
    )?;
    fs::write(root.join("diagrams/decision-policy.mmd"), decision_policy())?;
    Ok(())
}

fn load_states(root: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    load_records(root, "states.json")
}

fn load_records(root: &Path, filename: &str) -> Result<Vec<Value>, Box<dyn Error>> {
    let path = root.join("data").join(filename);
    let text = fs::read_to_string(path)?;
    let document: Value = serde_json::from_str(&text)?;
    document
        .get("records")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "states.records is missing"))
        .map_err(Into::into)
}

fn validate_record_count(
    root: &Path,
    filename: &str,
    expected: usize,
) -> Result<(), Box<dyn Error>> {
    let records = load_records(root, filename)?;
    if records.len() == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{filename} expected {expected} records, found {}",
                records.len()
            ),
        )
        .into())
    }
}

fn validate_state_ids(records: &[Value]) -> Result<(), Box<dyn Error>> {
    if records.len() != EXPECTED_STATES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected {EXPECTED_STATES} states, found {}", records.len()),
        )
        .into());
    }
    let mut ids = HashSet::with_capacity(records.len());
    for record in records {
        let state_id = required_string(record, "state_id")?;
        if safe_slug(state_id) != state_id {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unsafe state ID").into());
        }
        if !ids.insert(state_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("duplicate state ID: {state_id}"),
            )
            .into());
        }
    }
    Ok(())
}

fn required_string<'a>(record: &'a Value, path: &str) -> Result<&'a str, Box<dyn Error>> {
    record
        .get(path)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("state record is missing non-empty {path}"),
            )
            .into()
        })
}

fn safe_slug(state_id: &str) -> String {
    state_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn string_at<'a>(record: &'a Value, path: &[&str]) -> &'a str {
    path.iter()
        .try_fold(record, |value, key| value.get(*key))
        .and_then(Value::as_str)
        .unwrap_or("unspecified")
}

fn scalar_at(record: &Value, path: &[&str]) -> String {
    path.iter()
        .try_fold(record, |value, key| value.get(*key))
        .map(|value| match value {
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::String(value) => value.clone(),
            _ => "unspecified".to_owned(),
        })
        .unwrap_or_else(|| "unspecified".to_owned())
}

fn list_at(record: &Value, path: &[&str]) -> Vec<String> {
    path.iter()
        .try_fold(record, |value, key| value.get(*key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn bullets(items: &[String]) -> String {
    if items.is_empty() {
        "- None recorded in the requirements baseline.".to_owned()
    } else {
        items
            .iter()
            .map(|item| format!("- `{item}`"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn render_state_report(record: &Value) -> Result<String, Box<dyn Error>> {
    let state_id = required_string(record, "state_id")?;
    let display_name = string_at(record, &["display_name"]);
    let family = string_at(record, &["family"]);
    let build_id = string_at(record, &["build_id"]);
    let validation = string_at(record, &["validation_status"]);
    let entry = list_at(record, &["identity", "entry_conditions"]);
    let exit = list_at(record, &["identity", "exit_conditions"]);
    let predecessors = list_at(record, &["identity", "predecessors"]);
    let successors = list_at(record, &["identity", "successors"]);
    let required = list_at(record, &["observations", "required_fields"]);
    let on_demand = list_at(record, &["observations", "on_demand_fields"]);
    let historical = list_at(record, &["observations", "historical_fields"]);
    let derived = list_at(record, &["observations", "derived_fields"]);
    let estimated = list_at(record, &["observations", "estimated_fields"]);
    let unavailable = list_at(record, &["observations", "unavailable_fields"]);
    let expert_use = list_at(record, &["expert_use"]);
    let actions = list_at(record, &["legal_actions"]);
    let transitions = list_at(record, &["expected_transitions"]);
    let memory = list_at(record, &["memory_writes"]);
    let safe_actions = list_at(record, &["recovery", "safe_actions"]);
    let objective = string_at(record, &["policy", "primary_objective"]);
    let freshness = string_at(record, &["observations", "freshness_policy"]);
    let source_policy = string_at(record, &["observations", "source_policy"]);
    let recovery_policy = string_at(record, &["recovery", "policy_id"]);
    let unknown_behavior = string_at(record, &["recovery", "unknown_state_behavior"]);

    Ok(format!(
        "# {display_name} (`{state_id}`)\n\n\
Generated by `{GENERATOR_VERSION}` from the package state inventory. This is a proposed\n\
requirements baseline, not a target-build observation.\n\n\
| Property | Value |\n| --- | --- |\n| Family | `{family}` |\n| Build context | `{build_id}` |\n| Validation status | `{validation}` |\n| Stable/actionable identity | `{}` / `{}` |\n| Parent | `{}` |\n\n\
## Entry, exit, and graph\n\n### Entry predicates\n\n{}\n\n### Exit predicates\n\n{}\n\n### Predecessors\n\n{}\n\n### Successors\n\n{}\n\n\
## Legitimate observations\n\n### Required now\n\n{}\n\n### Ordinary on-demand inspection\n\n{}\n\n### Observed history\n\n{}\n\n### Derived exact\n\n{}\n\n### Estimated\n\n{}\n\n### Unavailable or denied\n\n{}\n\n\
Freshness: `{freshness}`\n\nSource boundary: `{source_policy}`\n\n\
## Expert use and policy\n\n{}\n\nPrimary objective: **{objective}**\n\nHard constraints and uncertainty treatment remain in the source state record and require\ntarget-build validation before implementation.\n\n\
## Candidate legal actions\n\n{}\n\nActions are semantic candidates generated from the current state. Raw coordinates, key codes,\nand invented operations are outside the policy boundary.\n\n\
## Expected transitions and verification\n\n{}\n\nThe action lifecycle is `requested -> accepted -> settled`; acceptance alone is not proof of\na game effect. Fresh successor observations and semantic postconditions are required.\n\n\
## Recovery\n\nPolicy: `{recovery_policy}`\n\nSafe actions:\n\n{}\n\nUnknown-state behavior: `{unknown_behavior}`\n\n\
## Memory and importance validation\n\nMemory writes:\n\n{}\n\nImportance labels are analyst priors only. No expert panel, target-build ablation, live trace,\nor simulator-parity result is claimed by this generated file.\n",
        scalar_at(record, &["identity", "stable"]),
        scalar_at(record, &["identity", "input_enabled"]),
        string_at(record, &["identity", "parent"]),
        bullets(&entry),
        bullets(&exit),
        bullets(&predecessors),
        bullets(&successors),
        bullets(&required),
        bullets(&on_demand),
        bullets(&historical),
        bullets(&derived),
        bullets(&estimated),
        bullets(&unavailable),
        bullets(&expert_use),
        bullets(&actions),
        bullets(&transitions),
        bullets(&safe_actions),
        bullets(&memory),
    ))
}

fn render_state_diagram(record: &Value) -> Result<String, Box<dyn Error>> {
    let state_id = required_string(record, "state_id")?;
    let display_name = string_at(record, &["display_name"]).replace('"', "'");
    let node_id = safe_slug(state_id);
    Ok(format!(
        "%% Generated by {GENERATOR_VERSION}; requirements baseline only.\nflowchart TD\n    E[\"{display_name}\"] --> B[\"stabilize and validate generation\"]\n    B --> L[\"enumerate semantic legal actions\"]\n    L --> X[\"select one catalog action\"]\n    X --> W[\"wait for fresh semantic successor\"]\n    W --> V{{\"postcondition verified?\"}}\n    V -->|yes| M[\"write bounded memory and replay record\"]\n    V -->|no| R[\"read-only reconciliation or safe halt\"]\n    R --> B\n    %% Stable node key: {node_id}\n",
    ))
}

fn global_architecture() -> &'static str {
    "%% Generated requirements diagram; no runtime/effect claim.\nflowchart LR\n    H[\"STS2 host/mod authority\"] --> F[\"fair-play projection\"]\n    F --> O[\"GameObservation\"]\n    O --> C[\"exact calculators + estimates\"]\n    C --> P[\"planner / policy\"]\n    P --> A[\"host-generated LegalAction\"]\n    A --> S[\"semantic adapter\"]\n    S --> H\n    H --> Q[\"fresh state + postcondition\"]\n    Q --> O\n    I[\"offline simulator / labels\"] -.\"isolated\".-> C\n    R[\"CV watchdog\"] -.\"independent parity\".-> O\n"
}

fn decision_policy() -> &'static str {
    "%% Generated requirements diagram; rationale is auditable, not hidden chain-of-thought.\nflowchart TD\n    E[\"state entry\"] --> B[\"stabilization barrier\"]\n    B --> V{\"fresh legitimate observation?\"}\n    V -->|no| R[\"re-observe / recover\"]\n    R --> B\n    V -->|yes| D[\"derived exact features\"]\n    D --> U[\"labeled estimates\"]\n    U --> L[\"authoritative legal-action catalog\"]\n    L --> S[\"hard constraints + sequence value\"]\n    S --> X[\"one semantic action\"]\n    X --> W[\"settlement wait\"]\n    W --> P{\"postcondition verified?\"}\n    P -->|yes| M[\"memory + replay\"]\n    P -->|no| R\n    Z[\"PRIVILEGED data\"] -.\"blocked\".-> X\n"
}
