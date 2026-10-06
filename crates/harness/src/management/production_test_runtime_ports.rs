// SPDX-License-Identifier: MIT

use super::Runtime;
use crate::episode::{
    BarrierError, BarrierPort, EpisodeObservation, RecoveryError, RecoveryPort, ShutdownError,
    ShutdownPort, TransitionReceipt, WaitSample,
};
impl BarrierPort for Runtime {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        Err(BarrierError::PortFailure)
    }
}

impl RecoveryPort for Runtime {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        Ok(self.observation.clone())
    }

    fn reconcile(&mut self, _operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        Err(RecoveryError::Unsupported)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        let counters = &mut *self.counters.lock().expect("counter lock");
        counters.stop_calls += 1;
        if counters.stop_fault_on_call == Some(counters.stop_calls) {
            return Err(RecoveryError::PortFailure);
        }
        Ok(())
    }
}

impl ShutdownPort for Runtime {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
}
