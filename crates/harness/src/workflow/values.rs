// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::ids::{ArtifactId, BoundedText, Digest, Generation};
use super::plans::{DecisionProposal, SubworkflowSelection};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ScalarValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Text(BoundedText),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationValue {
    pub state_id: BoundedText,
    pub generation: Generation,
    pub fields: BTreeMap<String, ScalarValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisValue {
    pub code: BoundedText,
    pub fields: BTreeMap<String, ScalarValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactValue {
    pub artifact_id: ArtifactId,
    pub digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TypedValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Text(BoundedText),
    Observation(ObservationValue),
    Analysis(AnalysisValue),
    DecisionProposal(Box<DecisionProposal>),
    SubworkflowSelection(Box<SubworkflowSelection>),
    Artifact(ArtifactValue),
    Unknown,
    Unavailable,
}

impl TypedValue {
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::Null => ValueType::Null,
            Self::Boolean(_) => ValueType::Boolean,
            Self::Integer(_) => ValueType::Integer,
            Self::Text(_) => ValueType::Text,
            Self::Observation(_) => ValueType::Observation,
            Self::Analysis(_) => ValueType::Analysis,
            Self::DecisionProposal(_) => ValueType::DecisionProposal,
            Self::SubworkflowSelection(_) => ValueType::SubworkflowSelection,
            Self::Artifact(_) => ValueType::Artifact,
            Self::Unknown => ValueType::Unknown,
            Self::Unavailable => ValueType::Unavailable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Null,
    Boolean,
    Integer,
    Text,
    Observation,
    Analysis,
    DecisionProposal,
    SubworkflowSelection,
    Artifact,
    Unknown,
    Unavailable,
}
