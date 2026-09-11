// SPDX-License-Identifier: MIT

// Resumable migration state with an explicit downgrade fence.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationController {
    journal: MigrationJournal,
    total_checkpoints: u64,
    fail_next_step: bool,
}

impl MigrationController {
    pub fn new(journal: MigrationJournal, total_checkpoints: u64) -> Result<Self, MemoryError> {
        journal.validate()?;
        if total_checkpoints == 0 || journal.checkpoint > total_checkpoints {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(Self {
            journal,
            total_checkpoints,
            fail_next_step: false,
        })
    }

    pub fn set_fail_next_step(&mut self, fail: bool) {
        self.fail_next_step = fail;
    }

    pub fn step(&mut self) -> Result<MigrationPhase, MemoryError> {
        if self.journal.phase == MigrationPhase::Complete {
            return Ok(MigrationPhase::Complete);
        }
        if self.fail_next_step {
            self.fail_next_step = false;
            self.journal.phase = MigrationPhase::Applying;
            return Err(MemoryError::PublicationFailed);
        }
        self.journal.phase = MigrationPhase::Applying;
        self.journal.checkpoint = self
            .journal
            .checkpoint
            .saturating_add(1)
            .min(self.total_checkpoints);
        if self.journal.checkpoint == self.total_checkpoints {
            self.journal.phase = MigrationPhase::Complete;
        }
        Ok(self.journal.phase)
    }

    pub fn resume(&mut self) -> Result<MigrationPhase, MemoryError> {
        self.step()
    }

    pub fn journal(&self) -> &MigrationJournal {
        &self.journal
    }
}
