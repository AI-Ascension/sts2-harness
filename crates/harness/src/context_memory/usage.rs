// SPDX-License-Identifier: MIT

// Usage is keyed by immutable provider attempts so retries and action fan-out cannot duplicate
// cached input or summary maintenance costs.

pub const MAX_USAGE_ATTEMPTS: usize = 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageLedger {
    attempts: BTreeMap<String, (LineageManifest, UsageMeasurement)>,
}

impl UsageLedger {
    pub fn new() -> Self {
        Self {
            attempts: BTreeMap::new(),
        }
    }

    pub fn record(
        &mut self,
        lineage: LineageManifest,
        usage: UsageMeasurement,
    ) -> Result<bool, MemoryError> {
        lineage.validate()?;
        let attempt = lineage
            .provider_attempt_id
            .clone()
            .ok_or(MemoryError::InvalidQuery)?;
        if usage.summary_calls == 0
            && usage.gameplay_calls == 0
            && usage.input_bytes == 0
            && usage.output_bytes == 0
            && usage.cached_input_bytes == 0
            && usage.maintenance_bytes == 0
            && usage.unknown_calls == 0
        {
            return Err(MemoryError::InvalidQuery);
        }
        if let Some((existing, _)) = self.attempts.get(&attempt) {
            return if existing == &lineage {
                Ok(false)
            } else {
                Err(MemoryError::Conflict)
            };
        }
        if self.attempts.len() >= MAX_USAGE_ATTEMPTS {
            return Err(MemoryError::Capacity);
        }
        self.attempts.insert(attempt, (lineage, usage));
        Ok(true)
    }

    pub fn aggregate(&self) -> UsageMeasurement {
        self.attempts.values().fold(UsageMeasurement::default(), |mut total, (_, usage)| {
            total.summary_calls = total.summary_calls.saturating_add(usage.summary_calls);
            total.gameplay_calls = total.gameplay_calls.saturating_add(usage.gameplay_calls);
            total.input_bytes = total.input_bytes.saturating_add(usage.input_bytes);
            total.output_bytes = total.output_bytes.saturating_add(usage.output_bytes);
            total.cached_input_bytes = total.cached_input_bytes.saturating_add(usage.cached_input_bytes);
            total.maintenance_bytes = total.maintenance_bytes.saturating_add(usage.maintenance_bytes);
            total.unknown_calls = total.unknown_calls.saturating_add(usage.unknown_calls);
            total
        })
    }

    pub fn len(&self) -> usize {
        self.attempts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attempts.is_empty()
    }
}

impl Default for UsageLedger {
    fn default() -> Self {
        Self::new()
    }
}
