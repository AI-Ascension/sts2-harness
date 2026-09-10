// SPDX-License-Identifier: MIT

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::super::runtime_v3_parse as parse;
use super::super::seed_transport::SeedTransportConfig;
use super::seeded_validation::{
    seeded_tool_value, validate_seeded_response, validate_seeded_settlement,
};
use super::{McpProcess, RuntimeV3Port, wire};

impl RuntimeV3Port {
    /// Read the live Runtime-v3 projection before a seeded mutation so the
    /// request fence is tied to the host's current generation. A newly
    /// constructed port starts at zero, but the host may already have advanced
    /// while it was entering its setup screen.
    pub(super) fn prime_seed_generation(&mut self) -> Result<(), String> {
        let (value, response_text) =
            self.call_tool_with_text("sts2.observe", self.context(self.generation))?;
        let parsed =
            parse::observation_with_text(&value, &response_text, "state_response", &self.config)?;
        let observation = self.install(parsed)?;
        if observation.generation() > super::super::seed_transport::SEEDED_RUN_MAX_GENERATION {
            return Err(String::from(
                "Runtime-v3 seed request generation exceeds the seeded-run bound",
            ));
        }
        Ok(())
    }

    /// Admit the selected game seed before opening the gameplay profile.  The
    /// start request is sent once; every later exchange is a read-only
    /// reconciliation for the same operation identity.
    pub(super) fn launch_seeded_run(&mut self) -> Result<(), String> {
        let Some(seed) = self.config.seed_transport.clone() else {
            return Ok(());
        };
        let request_generation = self.generation;
        let reservation = seed.reserve(request_generation)?;
        let mut mcp = McpProcess::spawn_profile(&self.config, "seeded-run-v1")?;
        if let Err(error) = wire::initialize_mcp_profile(&mut mcp, "seeded-run-v1") {
            let _ = mcp.close();
            return Err(error);
        }
        let operation_id = seed.operation_id.clone();
        let start_arguments = seed.start_arguments(
            &self.config.instance_id,
            &self.config.mcp_session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
            request_generation,
        );
        let mut receipt = json!({
            "operation_id": operation_id,
            "requested_seed": seed.requested_seed,
            "plan_digest": seed.plan_digest,
            "entry_ordinal": seed.entry_ordinal,
            "start": Value::Null,
            "reconcile": [],
        });
        if reservation.resumed {
            let settled = self.reconcile_until_seeded_settled(
                &mut mcp,
                &seed,
                &mut receipt,
                2,
                reservation.request_generation(),
            )?;
            validate_seeded_settlement(
                &settled,
                &self.config,
                &seed,
                reservation.request_generation(),
                "reconcile_response",
            )?;
            self.install_seeded_receipt(receipt, settled, &reservation, &seed)?;
            self.seeded_mcp = Some(mcp);
            return Ok(());
        }
        let start = mcp.call(
            1,
            "tools/call",
            json!({"name": "start_seeded_run", "arguments": start_arguments.clone()}),
        );
        let (start_value, start_kind) = match start {
            Ok(value) => {
                receipt["start"] = value.clone();
                let value = seeded_tool_value(value)?;
                validate_seeded_response(
                    &value,
                    &self.config,
                    &seed,
                    request_generation,
                    "start_response",
                )?;
                (value, "start_response")
            }
            Err(error) => {
                receipt["start_error"] = Value::String(error);
                let _ = reservation.mark_unknown(&seed);
                // McpProcess terminates a child on an uncertain exchange. A
                // fresh seeded profile is safe here because reconciliation is
                // a GET and carries the original operation ID.
                let _ = mcp.close();
                mcp = self.reconcile_seeded_process()?;
                let value = self.reconcile_until_seeded_settled(
                    &mut mcp,
                    &seed,
                    &mut receipt,
                    2,
                    request_generation,
                )?;
                (value, "reconcile_response")
            }
        };
        if start_value["status"] == "settled" {
            validate_seeded_settlement(
                &start_value,
                &self.config,
                &seed,
                request_generation,
                start_kind,
            )?;
            self.verify_seeded_idempotency(
                &mut mcp,
                &seed,
                &start_arguments,
                &start_value,
                &mut receipt,
                2,
            )?;
            let reconciled = self.reconcile_until_seeded_settled(
                &mut mcp,
                &seed,
                &mut receipt,
                3,
                request_generation,
            )?;
            validate_seeded_settlement(
                &reconciled,
                &self.config,
                &seed,
                request_generation,
                "reconcile_response",
            )?;
            self.install_seeded_receipt(receipt, reconciled, &reservation, &seed)?;
            self.seeded_mcp = Some(mcp);
            return Ok(());
        }
        if matches!(
            start_value["status"].as_str(),
            Some("rejected" | "cancelled")
        ) {
            let status = start_value["status"].as_str().unwrap_or("unknown");
            let _ = mcp.close();
            self.emit_seeded_receipt(receipt);
            return Err(format!("seeded run was {status}"));
        }
        let settled = self.reconcile_until_seeded_settled(
            &mut mcp,
            &seed,
            &mut receipt,
            2,
            request_generation,
        )?;
        validate_seeded_settlement(
            &settled,
            &self.config,
            &seed,
            request_generation,
            "reconcile_response",
        )?;
        self.verify_seeded_idempotency(
            &mut mcp,
            &seed,
            &start_arguments,
            &settled,
            &mut receipt,
            3,
        )?;
        self.install_seeded_receipt(receipt, settled, &reservation, &seed)?;
        self.seeded_mcp = Some(mcp);
        Ok(())
    }

    fn reconcile_seeded_process(&self) -> Result<McpProcess, String> {
        let mut mcp = McpProcess::spawn_profile(&self.config, "seeded-run-v1")?;
        if let Err(error) = wire::initialize_mcp_profile(&mut mcp, "seeded-run-v1") {
            let _ = mcp.close();
            return Err(error);
        }
        Ok(mcp)
    }

    fn verify_seeded_idempotency(
        &self,
        mcp: &mut McpProcess,
        seed: &SeedTransportConfig,
        start_arguments: &Value,
        settled: &Value,
        receipt: &mut Value,
        request_id: u64,
    ) -> Result<(), String> {
        if !seed.verify_idempotency {
            return Ok(());
        }
        let request_generation = start_arguments["generation"]
            .as_u64()
            .ok_or_else(|| String::from("seeded-run start arguments omitted generation"))?;
        let duplicate = mcp.call(
            request_id,
            "tools/call",
            json!({"name": "start_seeded_run", "arguments": start_arguments}),
        )?;
        receipt["duplicate_start"] = duplicate.clone();
        let duplicate_value = seeded_tool_value(duplicate)?;
        validate_seeded_settlement(
            &duplicate_value,
            &self.config,
            seed,
            request_generation,
            "start_response",
        )?;
        for field in ["canonical_seed", "observation", "effect_witness"] {
            if duplicate_value[field] != settled[field] {
                return Err(format!(
                    "seeded-run duplicate start changed the {field} witness"
                ));
            }
        }
        Ok(())
    }

    fn reconcile_until_seeded_settled(
        &self,
        mcp: &mut McpProcess,
        seed: &SeedTransportConfig,
        receipt: &mut Value,
        mut request_id: u64,
        request_generation: u64,
    ) -> Result<Value, String> {
        let deadline =
            Instant::now().checked_add(Duration::from_secs(self.config.settlement_timeout_seconds));
        loop {
            let arguments = json!({
                "instance_id": self.config.instance_id,
                "mcp_session_id": self.config.mcp_session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "generation": request_generation,
                "operation_id": seed.operation_id,
            });
            let response = mcp.call(
                request_id,
                "tools/call",
                json!({"name": "reconcile_seeded_run", "arguments": arguments}),
            )?;
            let value = seeded_tool_value(response.clone())?;
            validate_seeded_response(
                &value,
                &self.config,
                seed,
                request_generation,
                "reconcile_response",
            )?;
            receipt["reconcile"]
                .as_array_mut()
                .ok_or_else(|| String::from("seeded receipt reconcile field is not an array"))?
                .push(response);
            if value["status"] == "settled"
                || matches!(value["status"].as_str(), Some("rejected" | "cancelled"))
            {
                return Ok(value);
            }
            if self.config.settlement_timeout_seconds == 0
                || deadline.is_some_and(|value| Instant::now() >= value)
            {
                self.emit_seeded_receipt(receipt.clone());
                return Err(String::from(
                    "seeded run remains unknown; reconcile with the same operation_id",
                ));
            }
            request_id = request_id.saturating_add(1);
            thread::sleep(Duration::from_millis(250));
        }
    }
}
