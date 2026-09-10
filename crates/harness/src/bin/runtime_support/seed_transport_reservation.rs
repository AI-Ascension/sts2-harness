// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use super::validation::required;
use super::{ReservationRecord, SEEDED_RUN_MAX_GENERATION, SeedReservation, SeedTransportConfig};

impl SeedReservation {
    fn write_record(&self, record: &ReservationRecord) -> Result<(), String> {
        let bytes = serde_json::to_vec(record).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("pending");
        let mut file = fs::File::create(&temporary)
            .map_err(|error| format!("seed reservation temp file failed: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("seed reservation write failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("seed reservation sync failed: {error}"))?;
        fs::rename(&temporary, &self.path)
            .map_err(|error| format!("seed reservation commit failed: {error}"))
    }

    pub(crate) fn mark_unknown(&self, seed: &SeedTransportConfig) -> Result<(), String> {
        self.write_record(&ReservationRecord {
            protocol: String::from("seeded-run-reservation-v1"),
            operation_id: seed.operation_id.clone(),
            plan_digest: seed.plan_digest.clone(),
            entry_ordinal: seed.entry_ordinal,
            requested_seed: seed.requested_seed.clone(),
            run_mode: seed.run_mode.clone(),
            context_digest: seed.context_digest().to_owned(),
            selected_context: seed.selected_context(),
            generation: self.generation,
            status: String::from("unknown"),
            settled_receipt: None,
        })
    }

    pub(crate) fn mark_settled(
        &self,
        seed: &SeedTransportConfig,
        receipt: Value,
    ) -> Result<(), String> {
        self.write_record(&ReservationRecord {
            protocol: String::from("seeded-run-reservation-v1"),
            operation_id: seed.operation_id.clone(),
            plan_digest: seed.plan_digest.clone(),
            entry_ordinal: seed.entry_ordinal,
            requested_seed: seed.requested_seed.clone(),
            run_mode: seed.run_mode.clone(),
            context_digest: seed.context_digest().to_owned(),
            selected_context: seed.selected_context(),
            generation: self.generation,
            status: String::from("settled"),
            settled_receipt: Some(receipt),
        })
    }

    pub(crate) fn request_generation(&self) -> u64 {
        self.generation
    }
}

impl SeedTransportConfig {
    /// Reserve the operation before creating an MCP process. A pre-existing
    /// record is valid only for the same operation identity and enters
    /// reconcile-only recovery, closing the crash window around POST delivery.
    pub(crate) fn reserve(&self, generation: u64) -> Result<SeedReservation, String> {
        if generation > SEEDED_RUN_MAX_GENERATION {
            return Err(String::from("seed request generation exceeds its bound"));
        }
        let raw_path = required("STS2_SEED_RESERVATION_PATH")?;
        if raw_path.len() > 512 || raw_path.chars().any(char::is_control) {
            return Err(String::from(
                "STS2_SEED_RESERVATION_PATH is unsafe or oversized",
            ));
        }
        self.reserve_at(PathBuf::from(raw_path), generation)
    }

    pub(super) fn reserve_at(
        &self,
        path: PathBuf,
        generation: u64,
    ) -> Result<SeedReservation, String> {
        let record = ReservationRecord {
            protocol: String::from("seeded-run-reservation-v1"),
            operation_id: self.operation_id.clone(),
            plan_digest: self.plan_digest.clone(),
            entry_ordinal: self.entry_ordinal,
            requested_seed: self.requested_seed.clone(),
            run_mode: self.run_mode.clone(),
            context_digest: self.context_digest().to_owned(),
            selected_context: self.selected_context(),
            generation,
            status: String::from("start_pending"),
            settled_receipt: None,
        };
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let bytes = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
                file.write_all(&bytes)
                    .map_err(|error| format!("seed reservation write failed: {error}"))?;
                file.sync_all()
                    .map_err(|error| format!("seed reservation sync failed: {error}"))?;
                Ok(SeedReservation {
                    path,
                    generation,
                    resumed: false,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let bytes = fs::read(&path)
                    .map_err(|read_error| format!("seed reservation read failed: {read_error}"))?;
                let existing: ReservationRecord = serde_json::from_slice(&bytes)
                    .map_err(|parse_error| format!("seed reservation is invalid: {parse_error}"))?;
                if existing.protocol != "seeded-run-reservation-v1"
                    || existing.operation_id != self.operation_id
                    || existing.plan_digest != self.plan_digest
                    || existing.entry_ordinal != self.entry_ordinal
                    || existing.requested_seed != self.requested_seed
                    || existing.run_mode != self.run_mode
                    || existing.context_digest != self.context_digest()
                    || existing.selected_context != self.selected_context()
                {
                    return Err(String::from(
                        "seed reservation identity conflicts with the requested operation",
                    ));
                }
                if existing.generation > SEEDED_RUN_MAX_GENERATION {
                    return Err(String::from(
                        "seed reservation generation exceeds its safe integer bound",
                    ));
                }
                if !matches!(
                    existing.status.as_str(),
                    "start_pending" | "unknown" | "settled"
                ) {
                    return Err(String::from(
                        "seed reservation has an unsupported lifecycle state",
                    ));
                }
                if (existing.status == "settled") != existing.settled_receipt.is_some() {
                    return Err(String::from(
                        "seed reservation lifecycle receipt is inconsistent",
                    ));
                }
                Ok(SeedReservation {
                    path,
                    generation: existing.generation,
                    resumed: true,
                })
            }
            Err(error) => Err(format!("seed reservation create failed: {error}")),
        }
    }
}
