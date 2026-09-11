// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::super::seed_transport::{SeedTransportConfig, validate_recorded_context};
use super::trace::ReplayTrace;

#[path = "runtime_v3_replay_seeded_receipt_validation.rs"]
mod seeded_receipt_validation;
use seeded_receipt_validation::{digest, id, num, seed, shape};

pub(super) struct SeededAdmission {
    operation_id: String,
    requested_seed: String,
    plan_digest: String,
    entry_ordinal: u64,
    run_mode: String,
    selected_context: Value,
    context_digest: String,
    settled: Value,
}

impl ReplayTrace {
    pub(super) fn admit_seeded_receipt(
        &self,
        seed: Option<&SeedTransportConfig>,
    ) -> Result<(), String> {
        admit(self.seeded_admission.as_ref(), seed)
    }
}

pub(super) fn admit(
    admission: Option<&SeededAdmission>,
    seed: Option<&SeedTransportConfig>,
) -> Result<(), String> {
    let Some(a) = admission else {
        return Ok(());
    };
    let s = seed.ok_or("seeded receipt replay requires a current seed configuration")?;
    if a.operation_id != s.operation_id()
        || a.requested_seed != s.requested_seed()
        || a.plan_digest != s.plan_digest()
        || a.entry_ordinal != s.entry_ordinal()
        || a.run_mode != s.run_mode()
        || a.context_digest != s.context_digest()
        || a.selected_context != s.selected_context()
    {
        return Err("seeded receipt does not match the current replay seed configuration".into());
    }
    Ok(())
}
pub(super) fn parse_seeded_admission(row: &Value) -> Result<SeededAdmission, String> {
    let r = row["receipt"]
        .as_object()
        .ok_or("seeded receipt is not an object")?;
    const F: [&str; 7] = [
        "operation_id",
        "requested_seed",
        "plan_digest",
        "entry_ordinal",
        "start",
        "reconcile",
        "settled",
    ];
    if row.as_object().is_none_or(|o| o.len() != 2)
        || !(r.len() == 7 || r.len() == 8)
        || F.iter().any(|f| !r.contains_key(*f))
        || r.keys()
            .any(|k| !F.contains(&k.as_str()) && k != "duplicate_start")
    {
        return Err("seeded receipt has an invalid shape".into());
    }
    let op = id(&row["receipt"]["operation_id"], "operation_id")?;
    let seed = seed(&row["receipt"]["requested_seed"], "requested_seed")?;
    let plan = digest(&row["receipt"]["plan_digest"], "plan_digest")?;
    let ordinal = row["receipt"]["entry_ordinal"]
        .as_u64()
        .ok_or("seeded receipt entry ordinal is invalid")?;
    let start = wrapped(&row["receipt"]["start"])?;
    let source = Source::new(&start.value, &op, &seed)?;
    if start.is_error || start.value["status"] != "accepted" {
        return Err("seeded receipt does not prove a fresh accepted start".into());
    }
    let rs = row["receipt"]["reconcile"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("seeded receipt omitted reconciliation")?;
    let mut settled = None;
    for (i, raw) in rs.iter().enumerate() {
        let v = wrapped(raw)?;
        source.check(&v.value, "reconcile_response")?;
        if v.is_error && v.value["status"] != "unknown" {
            return Err("seeded receipt MCP error wrapper has an invalid status".into());
        }
        if i + 1 == rs.len() {
            if v.is_error || v.value["status"] != "settled" {
                return Err("seeded receipt reconciliation did not settle".into());
            }
            settled = Some(v.value);
        } else if !matches!(v.value["status"].as_str(), Some("accepted" | "unknown")) {
            return Err("seeded receipt contains contradictory reconciliation".into());
        }
    }
    let settled = settled.ok_or("seeded receipt omitted settlement")?;
    if row["receipt"]["settled"] != settled {
        return Err("seeded receipt settled record does not match reconciliation".into());
    }
    fresh(&settled, &source)?;
    if let Some(raw) = row["receipt"].get("duplicate_start") {
        let v = wrapped(raw)?;
        source.check(&v.value, "start_response")?;
        if v.is_error
            || v.value["status"] != "settled"
            || v.value["canonical_seed"] != settled["canonical_seed"]
            || v.value["observation"] != settled["observation"]
            || v.value["effect_witness"] != settled["effect_witness"]
        {
            return Err("seeded receipt duplicate start is not the same settled operation".into());
        }
    }
    Ok(SeededAdmission {
        operation_id: op,
        requested_seed: seed,
        plan_digest: plan,
        entry_ordinal: ordinal,
        run_mode: source.run_mode,
        selected_context: source.context,
        context_digest: source.context_digest,
        settled,
    })
}
pub(super) fn validate_first(
    a: &SeededAdmission,
    first: &Value,
    terminal: &Value,
) -> Result<(), String> {
    let g = num(
        &a.settled["observation"]["generation"],
        "settlement generation",
    )?;
    if first["state"]["state"] != "map"
        || first["visible_seed"] != a.settled["canonical_seed"]
        || num(&first["generation"], "first model observation generation")? != g
        || terminal["visible_seed"] != a.settled["canonical_seed"]
    {
        return Err(
            "seeded receipt replay does not begin at the settled new-run map boundary".into(),
        );
    }
    Ok(())
}
struct Source {
    instance: String,
    session: String,
    lease: String,
    epoch: u64,
    generation: u64,
    operation: String,
    requested: String,
    run_mode: String,
    context: Value,
    context_digest: String,
}
impl Source {
    fn new(v: &Value, op: &str, requested: &str) -> Result<Self, String> {
        shape(v, "start_response")?;
        let s = Self {
            instance: id(&v["instance_id"], "instance_id")?,
            session: id(&v["session_id"], "session_id")?,
            lease: id(&v["lease_id"], "lease_id")?,
            epoch: num(&v["lease_epoch"], "lease_epoch")?,
            generation: num(&v["generation"], "generation")?,
            operation: id(&v["operation_id"], "operation_id")?,
            requested: seed(&v["requested_seed"], "requested_seed")?,
            run_mode: id(&v["run_mode"], "run_mode")?,
            context: v["selected_context"].clone(),
            context_digest: digest(&v["context_digest"], "context_digest")?,
        };
        if s.operation != op
            || s.requested != requested
            || s.context["context_digest"] != s.context_digest
        {
            return Err("seeded receipt source lineage is invalid".into());
        }
        validate_recorded_context(&s.context, &s.context_digest)?;
        Ok(s)
    }
    fn check(&self, v: &Value, kind: &str) -> Result<(), String> {
        shape(v, kind)?;
        if v["instance_id"] != self.instance
            || v["session_id"] != self.session
            || v["lease_id"] != self.lease
            || v["lease_epoch"] != self.epoch
            || v["generation"] != self.generation
            || v["operation_id"] != self.operation
            || v["requested_seed"] != self.requested
            || v["run_mode"] != self.run_mode
            || v["selected_context"] != self.context
            || v["context_digest"] != self.context_digest
        {
            return Err("seeded receipt response changed original operation lineage".into());
        }
        Ok(())
    }
}
struct WrappedResponse {
    value: Value,
    is_error: bool,
}

fn wrapped(v: &Value) -> Result<WrappedResponse, String> {
    if v.as_object().is_none_or(|o| o.len() != 3)
        || v["jsonrpc"] != "2.0"
        || !v["id"].is_u64()
        || v["result"].as_object().is_none_or(|o| o.len() != 2)
        || !v["result"]["isError"].is_boolean()
        || v["result"]["content"]
            .as_array()
            .is_none_or(|a| a.len() != 1)
        || v["result"]["content"][0]
            .as_object()
            .is_none_or(|o| o.len() != 2)
        || v["result"]["content"][0]["type"] != "text"
        || !v["result"]["content"][0]["text"].is_string()
    {
        return Err("seeded receipt MCP wrapper is invalid".into());
    }
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .ok_or("seeded receipt MCP wrapper is invalid")?;
    let value = serde_json::from_str(text)
        .map_err(|_| String::from("seeded receipt MCP wrapper text is invalid"))?;
    Ok(WrappedResponse {
        value,
        is_error: v["result"]["isError"] == true,
    })
}
fn fresh(v: &Value, s: &Source) -> Result<(), String> {
    let seed = seed(&v["canonical_seed"], "canonical_seed")?;
    let o = v["observation"]
        .as_object()
        .ok_or("seeded receipt settlement observation is invalid")?;
    let w = v["effect_witness"]
        .as_object()
        .ok_or("seeded receipt settlement witness is invalid")?;
    if o.len() != 8
        || o["run_started"] != true
        || o["host_ready"] != true
        || o["canonical_seed"] != seed
        || o["selected_context_digest"] != s.context_digest
        || o["phase_before"] != "campaign_setup"
        || o["phase_after"] != "run_started"
        || num(&o["generation"], "settlement generation")? <= s.generation
        || w.len() != 3
        || w["kind"] != "run_started"
        || w["canonical_seed"] != seed
        || w["generation"] != o["generation"]
    {
        return Err("seeded receipt does not prove a fresh campaign setup to run start".into());
    }
    id(&o["compatibility_identity"], "compatibility_identity").map(|_| ())
}
