// SPDX-License-Identifier: MIT

//! Deterministic, auditable initial heuristic. Thresholds require held-out gameplay calibration.

use super::{AXES, Error, PROFILE, refusal, response};
use serde_json::{Value, json};

const MIN_EVIDENCE: f64 = 0.8;
const MIN_MARGIN: f64 = 0.1;
const MIN_SAFETY: f64 = 0.5;

pub(crate) fn evaluate(body: &Value, reply: &Value, gate: f64) -> Result<Value, Error> {
    if !gate.is_finite() || !(0.0..=1.0).contains(&gate) {
        return Err(Error::Gate);
    }
    let mut rows = response::rows(body, reply)?;
    rows.sort_by(|left, right| {
        right
            .utility
            .total_cmp(&left.utility)
            .then(left.action_id.cmp(&right.action_id))
    });
    let first = rows.first().ok_or(Error::Answers)?;
    let margin = rows
        .get(1)
        .map_or(1.0, |second| first.utility - second.utility);
    let decision = if rows.iter().any(|row| row.evidence < MIN_EVIDENCE) {
        refusal("missing material evidence; obtain a fresh observation or escalate")
    } else if first.min_confidence < gate || first.scores[5] < MIN_SAFETY || margin < MIN_MARGIN {
        refusal("uncertain tactical tradeoff; bounded re-observation or escalation required")
    } else {
        json!({
            "decision": "action", "action_id": first.action_id,
            "confidence": (first.min_confidence * 100.0).round() as u64,
            "rationale": format!("bridge-authored tactical heuristic: utility {:.3}, margin {:.3}; not a win probability", first.utility, margin),
        })
    };
    Ok(json!({
        "profile": PROFILE, "decision": decision, "rows": rows,
        "axes": AXES.iter().map(|axis| axis.0).collect::<Vec<_>>(),
        "weights": AXES.iter().map(|axis| axis.2).collect::<Vec<_>>(),
        "minimum_evidence": MIN_EVIDENCE, "minimum_margin": MIN_MARGIN,
        "minimum_safety": MIN_SAFETY, "confidence_gate": gate,
        "baseline_choice": reply["answers"]["action"]["choice"],
        "response_model": reply["model"],
    }))
}
