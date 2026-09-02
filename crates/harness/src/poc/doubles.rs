// SPDX-License-Identifier: MIT

use super::contract::{
    PocAction, PocCoreError, PocError, PocObservation, PocRequest, PocResponse, PocRoute, PocStatus,
};

const INSTANCE_ID: &str = "instance-1";
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
    const fn new() -> Self {
        Self {
            state: PocCoreState {
                generation: 0,
                available_units: 3,
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
    const fn new() -> Self {
        Self {
            core: CoreDouble::new(),
        }
    }

    fn forward(&mut self, request: PocRequest) -> Result<PocResponse, PocError> {
        if !request.is_valid(INSTANCE_ID, LEASE_ID) {
            return Err(PocError::InvalidRequest("mod metadata or shape is invalid"));
        }
        let state = self.core.read();
        match request.route() {
            PocRoute::State => Ok(PocResponse::state(
                request.correlation_id(),
                request.instance_id(),
                state.generation,
                state.observation(),
            )),
            PocRoute::Action => self.action(request, state),
        }
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

#[derive(Debug)]
struct GatewayDouble {
    instance_id: &'static str,
    lease_id: &'static str,
    game_mod: GameModDouble,
}

impl GatewayDouble {
    const fn new() -> Self {
        Self {
            instance_id: INSTANCE_ID,
            lease_id: LEASE_ID,
            game_mod: GameModDouble::new(),
        }
    }

    fn forward(&mut self, request: PocRequest) -> Result<PocResponse, PocError> {
        if request.instance_id() != self.instance_id {
            return Err(PocError::GatewayFence("wrong instance"));
        }
        if request.lease_id() != self.lease_id {
            return Err(PocError::GatewayFence("stale lease"));
        }
        self.game_mod.forward(request)
    }
}

#[derive(Debug)]
pub(super) struct McpDouble {
    gateway: GatewayDouble,
}

impl McpDouble {
    pub(super) const fn new() -> Self {
        Self {
            gateway: GatewayDouble::new(),
        }
    }

    pub(super) fn get_state(
        &mut self,
        _session_id: &str,
        instance_id: &str,
        correlation_id: &str,
    ) -> Result<PocResponse, PocError> {
        self.gateway
            .forward(PocRequest::state(correlation_id, instance_id, LEASE_ID))
    }

    pub(super) fn submit_action(
        &mut self,
        _session_id: &str,
        instance_id: &str,
        correlation_id: &str,
        generation: u64,
        action_id: &'static str,
        units: u16,
    ) -> Result<PocResponse, PocError> {
        self.gateway.forward(PocRequest::action_request(
            correlation_id,
            instance_id,
            generation,
            PocAction::new(action_id, units),
            LEASE_ID,
        ))
    }
}
