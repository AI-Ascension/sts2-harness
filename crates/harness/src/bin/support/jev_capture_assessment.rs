// SPDX-License-Identifier: MIT

//! Sanitized numeric diagnostics only. All strings are selected from local closed vocabularies.

use super::{Error, index};
use serde_json::{Value, json};

pub(super) fn project(record: &Value, catalog: &[String]) -> Result<Value, Error> {
    let Some(tactical) = record.get("tactical") else {
        return Ok(Value::Null);
    };
    let applied = tactical["applied"].as_bool().ok_or(Error::Evidence)?;
    if !applied {
        let reason = tactical["fallback_reason"]
            .as_str()
            .ok_or(Error::Evidence)?;
        if !matches!(
            reason,
            "forced_action"
                | "legacy_forced_group"
                | "candidate_bound_or_incomplete_catalog"
                | "no_comparison_needed"
                | "question_batch_budget"
        ) {
            return Err(Error::Evidence);
        }
        return Ok(json!({"applied": false, "fallback_reason": reason}));
    }
    let assessment = &tactical["assessment"];
    let source = assessment["rows"].as_array().ok_or(Error::Evidence)?;
    if source.is_empty() || source.len() > 24 {
        return Err(Error::Evidence);
    }
    let rows = source
        .iter()
        .map(|row| project_row(row, catalog))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "applied": true, "rows": rows,
        "within_request_index": index(catalog, assessment.get("baseline_choice"))?.ok_or(Error::Evidence)?,
        "minimum_evidence": unit(&assessment["minimum_evidence"] )?,
        "minimum_margin": unit(&assessment["minimum_margin"] )?,
        "minimum_safety": unit(&assessment["minimum_safety"] )?,
    }))
}

fn project_row(row: &Value, catalog: &[String]) -> Result<Value, Error> {
    let source = row["scores"].as_array().ok_or(Error::Evidence)?;
    if source.len() != 6 {
        return Err(Error::Evidence);
    }
    let scores = source.iter().map(unit).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "index": index(catalog, row.get("action_id"))?.ok_or(Error::Evidence)?,
        "scores": scores, "evidence": unit(&row["evidence"] )?,
        "min_confidence": unit(&row["min_confidence"] )?,
        "utility": unit(&row["utility"] )?,
    }))
}

fn unit(value: &Value) -> Result<f64, Error> {
    value
        .as_f64()
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or(Error::Evidence)
}
