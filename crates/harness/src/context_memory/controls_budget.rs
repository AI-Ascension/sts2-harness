// SPDX-License-Identifier: MIT

pub const MAX_GLOBAL_MEMORY_BYTES: usize = 256 * 1024;
pub const MAX_GLOBAL_MEMORY_JOBS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationState {
    Reserved,
    Unknown,
    Completed,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBudgetReservation {
    pub reservation_id: String,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub state: ReservationState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryBudgetLedger {
    max_jobs: usize,
    max_bytes: usize,
    reservations: BTreeMap<String, MemoryBudgetReservation>,
    reserved_bytes: usize,
}

impl MemoryBudgetLedger {
    pub fn new(max_jobs: usize, max_bytes: usize) -> Result<Self, MemoryError> {
        if max_jobs == 0
            || max_jobs > MAX_GLOBAL_MEMORY_JOBS
            || max_bytes == 0
            || max_bytes > MAX_GLOBAL_MEMORY_BYTES
        {
            return Err(MemoryError::Capacity);
        }
        Ok(Self {
            max_jobs,
            max_bytes,
            reservations: BTreeMap::new(),
            reserved_bytes: 0,
        })
    }

    pub fn reserve(
        &mut self,
        reservation_id: impl Into<String>,
        input_bytes: usize,
        output_bytes: usize,
    ) -> Result<MemoryBudgetReservation, MemoryError> {
        let reservation_id = reservation_id.into();
        if !valid_id(&reservation_id)
            || input_bytes > MAX_JOB_INPUT_BYTES
            || output_bytes == 0
            || output_bytes > MAX_SUMMARY_OUTPUT_BYTES
        {
            return Err(MemoryError::BudgetExceeded);
        }
        let bytes = input_bytes.saturating_add(output_bytes);
        if let Some(existing) = self.reservations.get(&reservation_id) {
            if existing.input_bytes == input_bytes
                && existing.output_bytes == output_bytes
                && existing.state != ReservationState::Released
            {
                return Ok(existing.clone());
            }
            return Err(MemoryError::Conflict);
        }
        if self
            .reservations
            .values()
            .filter(|item| {
                matches!(item.state, ReservationState::Reserved | ReservationState::Unknown)
            })
            .count()
            >= self.max_jobs
            || self.reserved_bytes.saturating_add(bytes) > self.max_bytes
        {
            return Err(MemoryError::BudgetExceeded);
        }
        let reservation = MemoryBudgetReservation {
            reservation_id: reservation_id.clone(),
            input_bytes,
            output_bytes,
            state: ReservationState::Reserved,
        };
        self.reserved_bytes = self.reserved_bytes.saturating_add(bytes);
        self.reservations.insert(reservation_id, reservation.clone());
        Ok(reservation)
    }

    pub fn mark_unknown(
        &mut self,
        reservation_id: &str,
    ) -> Result<MemoryBudgetReservation, MemoryError> {
        let reservation = self
            .reservations
            .get_mut(reservation_id)
            .ok_or(MemoryError::JobUnknown)?;
        if reservation.state != ReservationState::Reserved {
            return Err(MemoryError::JobUnknown);
        }
        reservation.state = ReservationState::Unknown;
        Ok(reservation.clone())
    }

    pub fn finish(
        &mut self,
        reservation_id: &str,
        terminal: ReservationState,
    ) -> Result<(), MemoryError> {
        if !matches!(terminal, ReservationState::Completed | ReservationState::Released) {
            return Err(MemoryError::JobUnknown);
        }
        let reservation = self
            .reservations
            .get_mut(reservation_id)
            .ok_or(MemoryError::JobUnknown)?;
        if matches!(reservation.state, ReservationState::Completed | ReservationState::Released) {
            return Ok(());
        }
        self.reserved_bytes = self
            .reserved_bytes
            .saturating_sub(reservation.input_bytes.saturating_add(reservation.output_bytes));
        reservation.state = terminal;
        Ok(())
    }

    pub fn reserved_bytes(&self) -> usize {
        self.reserved_bytes
    }

    pub fn active_jobs(&self) -> usize {
        self.reservations
            .values()
            .filter(|item| {
                matches!(item.state, ReservationState::Reserved | ReservationState::Unknown)
            })
            .count()
    }

    pub fn reservation(&self, reservation_id: &str) -> Option<&MemoryBudgetReservation> {
        self.reservations.get(reservation_id)
    }
}
