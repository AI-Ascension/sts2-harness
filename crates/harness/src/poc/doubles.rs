// SPDX-License-Identifier: MIT

use super::contract::{
    PocAction, PocCoreError, PocError, PocObservation, PocRequest, PocRoute, PocStatus,
};
use super::response::PocResponse;
use super::trace::TraceLedger;

const INSTANCE_ID: &str = "instance-1";
const SESSION_ID: &str = "session-1";
const LEASE_ID: &str = "lease-1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PocCoreState {
    generation: u64,
    available_units: u16,
    settled_effects: u16,
}

impl PocCoreState {
    const fn observation(self) -> PocObservation {
        PocObservation {
            available_units: self.available_units,
            settled_effects: self.settled_effects,
        }
    }
}

#[derive(Debug)]
struct CoreDouble {
    state: PocCoreState,
}

impl CoreDouble {
    const fn new(seed: u64, clock_tick: u64) -> Self {
        Self {
            state: PocCoreState {
                generation: 0,
                available_units: 2 + ((seed.wrapping_add(clock_tick) % 2) as u16),
                settled_effects: 0,
            },
        }
    }

    const fn read(&self) -> PocCoreState {
        self.state
    }

    fn apply(&mut self, generation: u64, action: PocAction) -> Result<PocCoreState, PocCoreError> {
        if generation != self.state.generation {
            return Err(PocCoreError::StaleGeneration);
        }
        if action.action_id() != "use_budget" {
            return Err(PocCoreError::InsufficientUnits);
        }
        if action.units() == 0 {
            return Err(PocCoreError::ZeroUnits);
        }
        if action.units() > self.state.available_units {
            return Err(PocCoreError::InsufficientUnits);
        }
        self.state = PocCoreState {
            generation: self.state.generation.saturating_add(1),
            available_units: self.state.available_units - action.units(),
            settled_effects: self.state.settled_effects.saturating_add(1),
        };
        Ok(self.state)
    }
}

#[derive(Debug)]
struct GameModDouble {
    core: CoreDouble,
}

impl GameModDouble {
    const fn new(seed: u64, clock_tick: u64) -> Self {
        Self {
            core: CoreDouble::new(seed, clock_tick),
        }
    }

    fn forward(
        &mut self,
        request: PocRequest,
        trace: &mut TraceLedger,
    ) -> Result<PocResponse, PocError> {
        if !request.is_valid(INSTANCE_ID, SESSION_ID, LEASE_ID) {
            return Err(PocError::InvalidRequest("mod metadata or shape is invalid"));
        }
        request.wire_json()?;
        let tool = match request.route() {
            PocRoute::State => "get_state",
            PocRoute::Action => "submit_action",
        };
        let mod_token = trace.enter("game-mod", tool, SESSION_ID, LEASE_ID)?;
        let core_token = trace.enter("game-core", tool, SESSION_ID, LEASE_ID)?;
        let state = self.core.read();
        let response = match request.route() {
            PocRoute::State => Ok(PocResponse::state(
                request.correlation_id(),
                request.instance_id(),
                state.generation,
                state.observation(),
            )),
            PocRoute::Action => self.action(request, state),
        }?;
        response.wire_json()?;
        trace.complete(core_token, &response)?;
        trace.complete(mod_token, &response)?;
        Ok(response)
    }

    fn action(
        &mut self,
        request: PocRequest,
        state: PocCoreState,
    ) -> Result<PocResponse, PocError> {
        let Some(action) = request.action() else {
            return Err(PocError::InvalidRequest(
                "action request has no typed action",
            ));
        };
        match self.core.apply(request.generation(), action) {
            Ok(output) => Ok(PocResponse::action_response(
                request.correlation_id(),
                request.instance_id(),
                output.generation,
                output.observation(),
                action,
                PocStatus::Accepted,
                None,
            )),
            Err(error) => Ok(PocResponse::action_response(
                request.correlation_id(),
                request.instance_id(),
                state.generation,
                state.observation(),
                action,
                PocStatus::Rejected,
                Some(error.code()),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_rejects_a_request_from_another_session() {
        let mut mcp = McpDouble::new(7, 0);
        let mut trace = TraceLedger::new();
        assert_eq!(
            trace.enter("harness", "get_state", SESSION_ID, LEASE_ID),
            Ok(0)
        );

        assert_eq!(
            mcp.get_state("session-other", INSTANCE_ID, "corr-0001", &mut trace),
            Err(PocError::GatewayFence("wrong session"))
        );
    }

    #[test]
    fn seed_and_clock_select_the_bounded_initial_state() {
        assert_eq!(CoreDouble::new(7, 0).read().available_units, 3);
        assert_ne!(
            CoreDouble::new(7, 0).read().available_units,
            CoreDouble::new(6, 0).read().available_units
        );
        assert_ne!(
            CoreDouble::new(7, 0).read().available_units,
            CoreDouble::new(7, 1).read().available_units
        );
    }
}

#[derive(Debug)]
struct GatewayDouble {
    instance_id: &'static str,
    session_id: &'static str,
    lease_id: &'static str,
    game_mod: GameModDouble,
}

impl GatewayDouble {
    const fn new(seed: u64, clock_tick: u64) -> Self {
        Self {
            instance_id: INSTANCE_ID,
            session_id: SESSION_ID,
            lease_id: LEASE_ID,
            game_mod: GameModDouble::new(seed, clock_tick),
        }
    }

    fn forward(
        &mut self,
        request: PocRequest,
        trace: &mut TraceLedger,
    ) -> Result<PocResponse, PocError> {
        if request.instance_id() != self.instance_id {
            return Err(PocError::GatewayFence("wrong instance"));
        }
        if request.session_id() != self.session_id {
            return Err(PocError::GatewayFence("wrong session"));
        }
        if request.lease_id() != self.lease_id {
            return Err(PocError::GatewayFence("stale lease"));
        }
        let tool = match request.route() {
            PocRoute::State => "get_state",
            PocRoute::Action => "submit_action",
        };
        let token = trace.enter("gateway", tool, SESSION_ID, LEASE_ID)?;
        let response = self.game_mod.forward(request, trace)?;
        trace.complete(token, &response)?;
        Ok(response)
    }
}

#[derive(Debug)]
pub(super) struct McpDouble {
    gateway: GatewayDouble,
}

impl McpDouble {
    pub(super) const fn new(seed: u64, clock_tick: u64) -> Self {
        Self {
            gateway: GatewayDouble::new(seed, clock_tick),
        }
    }

    pub(super) fn get_state(
        &mut self,
        session_id: &str,
        instance_id: &str,
        correlation_id: &str,
        trace: &mut TraceLedger,
    ) -> Result<PocResponse, PocError> {
        let request = PocRequest::state(correlation_id, instance_id, session_id, LEASE_ID);
        request.wire_json()?;
        let token = trace.enter("mcp", "get_state", SESSION_ID, LEASE_ID)?;
        let response = self.gateway.forward(request, trace)?;
        response.wire_json()?;
        trace.complete(token, &response)?;
        Ok(response)
    }

    pub(super) fn submit_action(
        &mut self,
        request: PocRequest,
        trace: &mut TraceLedger,
    ) -> Result<PocResponse, PocError> {
        request.wire_json()?;
        let token = trace.enter("mcp", "submit_action", SESSION_ID, LEASE_ID)?;
        let response = self.gateway.forward(request, trace)?;
        response.wire_json()?;
        trace.complete(token, &response)?;
        Ok(response)
    }
}
