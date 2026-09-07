// SPDX-License-Identifier: MIT

use super::bundle::MapBundleError;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapEvaluationError {
    InvalidTasks,
    InvalidDecision,
    IncompleteMatrix(String),
    TooManyDecisions,
    TooLarge(&'static str),
    Serialization,
    UnknownTask(String),
    Bundle(String),
}

impl From<MapBundleError> for MapEvaluationError {
    fn from(error: MapBundleError) -> Self {
        Self::Bundle(error.to_string())
    }
}

impl fmt::Display for MapEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTasks => formatter.write_str("invalid synthetic map task"),
            Self::InvalidDecision => formatter.write_str("invalid synthetic map decision"),
            Self::IncompleteMatrix(reason) => {
                write!(
                    formatter,
                    "incomplete synthetic map evaluation matrix: {reason}"
                )
            }
            Self::TooManyDecisions => formatter.write_str("too many synthetic map decisions"),
            Self::TooLarge(field) => write!(formatter, "synthetic map {field} exceeds its bound"),
            Self::Serialization => formatter.write_str("synthetic map serialization failed"),
            Self::UnknownTask(task) => write!(formatter, "unknown synthetic map task {task}"),
            Self::Bundle(error) => write!(formatter, "synthetic map bundle rejected: {error}"),
        }
    }
}

impl std::error::Error for MapEvaluationError {}
