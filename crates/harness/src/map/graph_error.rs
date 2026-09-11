// SPDX-License-Identifier: MIT

use super::graph::{MAP_MAX_EDGES, MAP_MAX_NODES};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapGraphError {
    InvalidField(&'static str),
    InvalidDigest,
    TooManyNodes(usize),
    TooManyEdges(usize),
    UnavailableHasGraph,
    DuplicateNode(String),
    DuplicateEdge { from: String, to: String },
    DuplicateTerminal(String),
    DuplicateLegalDestination(String),
    UnknownNode(String),
    UnknownEndpoint { from: String, to: String },
    Wire(&'static str),
}

impl fmt::Display for MapGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid map field {field}"),
            Self::InvalidDigest => formatter.write_str("snapshot digest must be lowercase SHA-256"),
            Self::TooManyNodes(count) => write!(
                formatter,
                "map has {count} nodes; maximum is {MAP_MAX_NODES}"
            ),
            Self::TooManyEdges(count) => write!(
                formatter,
                "map has {count} edges; maximum is {MAP_MAX_EDGES}"
            ),
            Self::UnavailableHasGraph => {
                formatter.write_str("unavailable map cannot carry graph data")
            }
            Self::DuplicateNode(id) => write!(formatter, "duplicate map node {id}"),
            Self::DuplicateEdge { from, to } => {
                write!(formatter, "duplicate map edge {from}->{to}")
            }
            Self::DuplicateTerminal(id) => write!(formatter, "duplicate terminal {id}"),
            Self::DuplicateLegalDestination(id) => {
                write!(formatter, "duplicate legal destination {id}")
            }
            Self::UnknownNode(id) => write!(formatter, "unknown map node {id}"),
            Self::UnknownEndpoint { from, to } => {
                write!(formatter, "unknown edge endpoint {from}->{to}")
            }
            Self::Wire(field) => write!(formatter, "invalid visible-map wire field {field}"),
        }
    }
}

impl std::error::Error for MapGraphError {}
