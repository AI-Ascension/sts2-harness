// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::seed_transport::{SeedReservation, SeedTransportConfig};
use super::RuntimeV3Port;

impl RuntimeV3Port {
    pub(super) fn install_seeded_receipt(
        &mut self,
        mut receipt: Value,
        settled: Value,
        reservation: &SeedReservation,
        seed: &SeedTransportConfig,
    ) -> Result<(), String> {
        receipt["settled"] = settled.clone();
        reservation.mark_settled(seed, receipt.clone())?;
        if let Some(generation) = settled["observation"]["generation"].as_u64() {
            self.generation = generation;
        }
        self.emit_seeded_receipt(receipt.clone());
        self.seeded_receipt = Some(receipt);
        Ok(())
    }

    pub(super) fn emit_seeded_receipt(&self, receipt: Value) {
        println!(
            "{}",
            json!({"event": "seeded_run_receipt", "receipt": receipt})
        );
    }
}
