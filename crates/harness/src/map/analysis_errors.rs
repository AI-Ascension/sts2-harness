// SPDX-License-Identifier: MIT

use super::canonical::CanonicalError;
use super::graph::MapGraphError;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapAnalysisError {
    InvalidConfig,
    InvalidPolicy,
    Unavailable,
    Cycle(Vec<String>),
    Serialization,
    DigestMismatch,
    Canonical(CanonicalError),
    Graph(MapGraphError),
}

impl fmt::Display for MapAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid map analysis configuration"),
            Self::InvalidPolicy => formatter.write_str("invalid map route policy"),
            Self::Unavailable => formatter.write_str("map snapshot is unavailable"),
            Self::Cycle(nodes) => write!(formatter, "map graph contains a cycle: {nodes:?}"),
            Self::Serialization => formatter.write_str("map analysis serialization failed"),
            Self::DigestMismatch => formatter.write_str("map analysis digest mismatch"),
            Self::Canonical(error) => error.fmt(formatter),
            Self::Graph(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MapAnalysisError {}

impl From<MapGraphError> for MapAnalysisError {
    fn from(value: MapGraphError) -> Self {
        Self::Graph(value)
    }
}
