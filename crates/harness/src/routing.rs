// SPDX-License-Identifier: MIT

use crate::error::PortError;
use crate::identity::{EpisodeId, GatewaySessionId, InstanceId, RunId};

const MAX_ROUTE_TOKEN_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteRequest {
    run_id: RunId,
    episode_id: EpisodeId,
    preferred_instance: Option<InstanceId>,
}

impl RouteRequest {
    #[must_use]
    pub const fn new(
        run_id: RunId,
        episode_id: EpisodeId,
        preferred_instance: Option<InstanceId>,
    ) -> Self {
        Self {
            run_id,
            episode_id,
            preferred_instance,
        }
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn episode_id(&self) -> EpisodeId {
        self.episode_id
    }

    #[must_use]
    pub const fn preferred_instance(&self) -> Option<InstanceId> {
        self.preferred_instance
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteToken(String);

impl RouteToken {
    pub fn new(value: impl Into<String>) -> Result<Self, PortError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_ROUTE_TOKEN_BYTES {
            return Err(PortError::new(
                "invalid_route_token",
                "route token must be nonempty and bounded",
                false,
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteBinding {
    run_id: RunId,
    episode_id: EpisodeId,
    instance_id: InstanceId,
    gateway_session_id: GatewaySessionId,
    route_token: RouteToken,
}

impl RouteBinding {
    #[must_use]
    pub const fn new(
        run_id: RunId,
        episode_id: EpisodeId,
        instance_id: InstanceId,
        gateway_session_id: GatewaySessionId,
        route_token: RouteToken,
    ) -> Self {
        Self {
            run_id,
            episode_id,
            instance_id,
            gateway_session_id,
            route_token,
        }
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn episode_id(&self) -> EpisodeId {
        self.episode_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn gateway_session_id(&self) -> GatewaySessionId {
        self.gateway_session_id
    }

    #[must_use]
    pub fn route_token(&self) -> &RouteToken {
        &self.route_token
    }
}

pub trait InstanceRouter {
    fn bind(&mut self, request: &RouteRequest) -> Result<RouteBinding, PortError>;

    fn unbind(&mut self, binding: &RouteBinding) -> Result<(), PortError>;

    fn close(&mut self) -> Result<(), PortError>;
}
