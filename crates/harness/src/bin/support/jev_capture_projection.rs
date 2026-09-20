// SPDX-License-Identifier: MIT

//! Allowlisted projection. No state, action strings, prompts, rationales, paths or error text.

use super::{Error, SCHEMA, gate, options};
use serde_json::{Value, json};

#[path = "jev_capture_assessment.rs"]
mod assessment;

pub(super) struct Identity {
    fields: Value,
    catalog: Vec<String>,
}

impl Identity {
    pub(super) fn new(
        input: &Value,
        options: &options::Options,
        bridge_digest: &str,
    ) -> Result<Self, Error> {
        if bridge_digest.len() != 64
            || !bridge_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Input);
        }
        let execution = input["model_execution_id"].as_str().ok_or(Error::Input)?;
        if execution.is_empty() || execution.len() > 240 {
            return Err(Error::Input);
        }
        let mut catalog = super::super::catalog(input).map_err(|_| Error::Input)?;
        super::super::tactical::validate_catalog(&catalog).map_err(|_| Error::Input)?;
        catalog.sort();
        let mut comparable = input.clone();
        comparable
            .as_object_mut()
            .ok_or(Error::Input)?
            .remove("model_execution_id");
        let fields = json!({
            "schema": SCHEMA,
            "profile": if options.tactical { "jev-tactical-v1" } else { "baseline" },
            "bridge_digest": bridge_digest,
            "model_execution_id_digest": fingerprint("model_execution_id", &json!(execution))?,
            "input_digest": fingerprint("bridge_input_without_execution_id", &comparable)?,
            "catalog_digest": fingerprint("sorted_host_catalog", &json!(catalog))?,
            "catalog_count": catalog.len(),
            "requested_model_digest": fingerprint("model", &json!(options.model))?,
            "confidence_gate": gate(options),
        });
        Ok(Self { fields, catalog })
    }

    pub(super) fn pending(&self) -> Value {
        let mut value = self.fields.clone();
        value["status"] = json!("pending");
        // Null means unknown after a crash, not zero calls or zero elapsed time.
        value["provider_attempts"] = Value::Null;
        value["elapsed_ms"] = Value::Null;
        value
    }

    pub(super) fn failed(&self, attempts: u32, elapsed: u64) -> Value {
        let mut value = self.pending();
        value["status"] = json!("failed");
        value["provider_attempts"] = json!(attempts);
        value["elapsed_ms"] = json!(elapsed);
        value
    }

    pub(super) fn complete(
        &self,
        record: &Value,
        attempts: u32,
        elapsed: u64,
    ) -> Result<Value, Error> {
        let called = record["provider_call"].as_bool().ok_or(Error::Evidence)?;
        if u32::from(called) != attempts || attempts > 1 {
            return Err(Error::Evidence);
        }
        let mut value = self.failed(attempts, elapsed);
        value["status"] = json!("complete");
        value["decision"] = self.decision(&record["decision"])?;
        value["provider"] = if called {
            self.provider(record)?
        } else {
            Value::Null
        };
        value["tactical"] = assessment::project(record, &self.catalog)?;
        Ok(value)
    }

    fn decision(&self, decision: &Value) -> Result<Value, Error> {
        let kind = decision["decision"].as_str().ok_or(Error::Evidence)?;
        if !matches!(kind, "action" | "reobserve") {
            return Err(Error::Evidence);
        }
        let selected = index(&self.catalog, decision.get("action_id"))?;
        let candidate = index(&self.catalog, decision.get("candidate_action_id"))?;
        if (kind == "action") != selected.is_some() || (kind == "action" && candidate.is_some()) {
            return Err(Error::Evidence);
        }
        Ok(json!({"kind": kind, "selected_index": selected, "candidate_index": candidate}))
    }

    fn provider(&self, record: &Value) -> Result<Value, Error> {
        let body = &record["provider_request"];
        let mut shared = body.clone();
        if record["tactical"]["applied"] == true {
            // This is the bridge's own newly built record, not imported untrusted capture data.
            shared["state"] = body["state"]["observation_and_derived_facts"].clone();
            shared["questions"] = json!({"action": body["questions"]["action"]});
        }
        let requested = fingerprint("model", &body["model"])?;
        if Some(requested.as_str()) != self.fields["requested_model_digest"].as_str() {
            return Err(Error::Evidence);
        }
        let reply = &record["provider_response"];
        let returned = reply["model"].as_str().filter(|name| !name.is_empty());
        Ok(json!({
            "request_digest": fingerprint("provider_request", body)?,
            "shared_request_digest": fingerprint("shared_provider_request", &shared)?,
            "question_set_digest": fingerprint("question_set", &body["questions"] )?,
            "response_model_digest": returned.map(|name| fingerprint("model", &json!(name))).transpose()?,
            "input_tokens": reply["usage"]["input_tokens"].as_u64().filter(|n| *n <= 9_007_199_254_740_991),
        }))
    }
}

pub(super) fn index(catalog: &[String], value: Option<&Value>) -> Result<Option<usize>, Error> {
    let Some(value) = value else { return Ok(None) };
    let id = value.as_str().ok_or(Error::Evidence)?;
    catalog
        .iter()
        .position(|candidate| candidate == id)
        .map(Some)
        .ok_or(Error::Evidence)
}

/// Domain-separated serde_json byte identity, NOT JavaScript canonicalization or anonymization.
pub(super) fn fingerprint(domain: &str, value: &Value) -> Result<String, Error> {
    let mut bytes = format!("ascension.jev-capture.v1/{domain}\0").into_bytes();
    bytes.extend(serde_json::to_vec(value).map_err(|_| Error::Evidence)?);
    Ok(sts2_harness::sha256_hex(bytes))
}
