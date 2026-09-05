// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

const MAX_PEERS: usize = 4;
const MAX_GENERATION: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopPeerRole {
    Local,
    Ally,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopSyncStatus {
    Synchronized,
    Disagreement,
    Disconnected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PeerState {
    role: CoopPeerRole,
    generation: u64,
    connected: bool,
}

/// Harness-side coordination gate. It can suspend policy mutation but cannot perform a mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopCoordinator {
    generation: u64,
    peers: BTreeMap<String, PeerState>,
}

impl CoopCoordinator {
    pub fn new(generation: u64) -> Result<Self, CoopError> {
        if generation > 9_007_199_254_740_991 {
            return Err(CoopError::InvalidGeneration);
        }
        Ok(Self {
            generation,
            peers: BTreeMap::new(),
        })
    }

    pub fn register(
        &mut self,
        peer_id: impl Into<String>,
        role: CoopPeerRole,
        generation: u64,
    ) -> Result<(), CoopError> {
        let peer_id = peer_id.into();
        if !valid_identity(&peer_id) || generation != self.generation {
            return Err(CoopError::InvalidPeer);
        }
        if self.peers.len() >= MAX_PEERS {
            return Err(CoopError::PeerCapacity);
        }
        if self.peers.contains_key(&peer_id) {
            return Err(CoopError::DuplicatePeer);
        }
        if role == CoopPeerRole::Local
            && self
                .peers
                .values()
                .any(|peer| peer.role == CoopPeerRole::Local)
        {
            return Err(CoopError::DuplicateLocalPeer);
        }
        self.peers.insert(
            peer_id,
            PeerState {
                role,
                generation,
                connected: true,
            },
        );
        Ok(())
    }

    pub fn report_generation(&mut self, peer_id: &str, generation: u64) -> Result<(), CoopError> {
        if generation > MAX_GENERATION {
            return Err(CoopError::InvalidGeneration);
        }
        let peer = self.peers.get_mut(peer_id).ok_or(CoopError::UnknownPeer)?;
        peer.generation = generation;
        Ok(())
    }

    pub fn disconnect(&mut self, peer_id: &str) -> Result<(), CoopError> {
        self.peers
            .get_mut(peer_id)
            .ok_or(CoopError::UnknownPeer)?
            .connected = false;
        Ok(())
    }

    pub fn reconnect(&mut self, peer_id: &str, generation: u64) -> Result<(), CoopError> {
        if generation > MAX_GENERATION {
            return Err(CoopError::InvalidGeneration);
        }
        let peer = self.peers.get_mut(peer_id).ok_or(CoopError::UnknownPeer)?;
        peer.generation = generation;
        peer.connected = true;
        Ok(())
    }

    #[must_use]
    pub fn status(&self) -> CoopSyncStatus {
        if self.peers.values().any(|peer| !peer.connected) {
            return CoopSyncStatus::Disconnected;
        }
        if self
            .peers
            .values()
            .any(|peer| peer.generation != self.generation)
        {
            return CoopSyncStatus::Disagreement;
        }
        CoopSyncStatus::Synchronized
    }

    pub fn authorize_local_action(
        &self,
        peer_id: &str,
        generation: u64,
        ally_target: Option<&str>,
    ) -> Result<(), CoopError> {
        if self.status() != CoopSyncStatus::Synchronized {
            return Err(CoopError::MutationSuspended);
        }
        if generation != self.generation {
            return Err(CoopError::StaleGeneration);
        }
        let peer = self.peers.get(peer_id).ok_or(CoopError::UnknownPeer)?;
        if peer.role != CoopPeerRole::Local {
            return Err(CoopError::NotLocalPeer);
        }
        if let Some(target) = ally_target {
            let Some(ally) = self.peers.get(target) else {
                return Err(CoopError::InvalidAllyTarget);
            };
            if ally.role != CoopPeerRole::Ally {
                return Err(CoopError::InvalidAllyTarget);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopError {
    InvalidGeneration,
    InvalidPeer,
    PeerCapacity,
    DuplicatePeer,
    DuplicateLocalPeer,
    UnknownPeer,
    MutationSuspended,
    StaleGeneration,
    NotLocalPeer,
    InvalidAllyTarget,
}

impl std::fmt::Display for CoopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidGeneration => "co-op generation is invalid",
            Self::InvalidPeer => "co-op peer identity or generation is invalid",
            Self::PeerCapacity => "co-op peer capacity is exhausted",
            Self::DuplicatePeer => "co-op peer is duplicated",
            Self::DuplicateLocalPeer => "co-op local peer is duplicated",
            Self::UnknownPeer => "co-op peer is unknown",
            Self::MutationSuspended => "co-op mutation is suspended",
            Self::StaleGeneration => "co-op action generation is stale",
            Self::NotLocalPeer => "only the local peer may submit a local action",
            Self::InvalidAllyTarget => "ally target is not a known ally",
        })
    }
}

impl std::error::Error for CoopError {}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
