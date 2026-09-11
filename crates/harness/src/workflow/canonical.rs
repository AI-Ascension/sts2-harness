// SPDX-License-Identifier: MIT

use serde::Serialize;
use serde_json::{Map, Number, Value};

use super::definition::WorkflowDefinition;
use super::ids::{Digest, MAX_SAFE_INTEGER};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    Serialization,
    FloatNotAllowed,
    NonFinite,
    UnsafeInteger,
    DepthExceeded,
}

impl std::fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Serialization => "workflow value could not be serialized",
            Self::FloatNotAllowed => "floating point values are not part of workflow v1",
            Self::NonFinite => "non-finite values are not part of workflow v1",
            Self::UnsafeInteger => "integer exceeds the workflow v1 bound",
            Self::DepthExceeded => "canonical value exceeds the workflow v1 depth bound",
        })
    }
}

impl std::error::Error for CanonicalError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticDiff {
    pub executable_changed: bool,
    pub annotations_changed: bool,
}

pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, CanonicalError> {
    let mut output = Vec::new();
    write_value(value, 0, &mut output)?;
    Ok(output)
}

pub fn semantic_diff(
    left: &WorkflowDefinition,
    right: &WorkflowDefinition,
) -> Result<SemanticDiff, CanonicalError> {
    let left_value = serde_json::to_value(left).map_err(|_| CanonicalError::Serialization)?;
    let right_value = serde_json::to_value(right).map_err(|_| CanonicalError::Serialization)?;
    let mut left_semantic = left_value.clone();
    let mut right_semantic = right_value.clone();
    remove_annotations(&mut left_semantic);
    remove_annotations(&mut right_semantic);
    let left_bytes = canonical_json_bytes(&left_semantic)?;
    let right_bytes = canonical_json_bytes(&right_semantic)?;
    Ok(SemanticDiff {
        executable_changed: left_bytes != right_bytes,
        annotations_changed: left_value.get("annotations") != right_value.get("annotations"),
    })
}

impl WorkflowDefinition {
    pub fn canonical_semantic_bytes(&self) -> Result<Vec<u8>, CanonicalError> {
        let mut value = serde_json::to_value(self).map_err(|_| CanonicalError::Serialization)?;
        remove_annotations(&mut value);
        canonical_json_bytes(&value)
    }

    pub fn semantic_digest(&self) -> Result<Digest, CanonicalError> {
        Ok(Digest::sha256(&self.canonical_semantic_bytes()?))
    }
}

fn remove_annotations(value: &mut Value) {
    if let Value::Object(object) = value {
        object.remove("annotations");
    }
}

fn write_value(value: &Value, depth: usize, output: &mut Vec<u8>) -> Result<(), CanonicalError> {
    if depth > 32 {
        return Err(CanonicalError::DepthExceeded);
    }
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(number) => write_number(number, output)?,
        Value::String(value) => write_json_fragment(value, output)?,
        Value::Array(values) => {
            output.push(b'[');
            for (index, child) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_value(child, depth + 1, output)?;
            }
            output.push(b']');
        }
        Value::Object(object) => write_object(object, depth, output)?,
    }
    Ok(())
}

fn write_object(
    object: &Map<String, Value>,
    depth: usize,
    output: &mut Vec<u8>,
) -> Result<(), CanonicalError> {
    let mut entries: Vec<(&String, &Value)> = object.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    output.push(b'{');
    for (index, (key, value)) in entries.into_iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        write_json_fragment(key, output)?;
        output.push(b':');
        write_value(value, depth + 1, output)?;
    }
    output.push(b'}');
    Ok(())
}

fn write_number(number: &Number, output: &mut Vec<u8>) -> Result<(), CanonicalError> {
    if let Some(value) = number.as_i64() {
        if value.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(CanonicalError::UnsafeInteger);
        }
    } else if let Some(value) = number.as_u64() {
        if value > MAX_SAFE_INTEGER {
            return Err(CanonicalError::UnsafeInteger);
        }
    } else if let Some(value) = number.as_f64() {
        if !value.is_finite() {
            return Err(CanonicalError::NonFinite);
        }
        return Err(CanonicalError::FloatNotAllowed);
    } else {
        return Err(CanonicalError::FloatNotAllowed);
    }
    output.extend_from_slice(number.to_string().as_bytes());
    Ok(())
}

fn write_json_fragment<T: Serialize>(
    value: &T,
    output: &mut Vec<u8>,
) -> Result<(), CanonicalError> {
    let bytes = serde_json::to_vec(value).map_err(|_| CanonicalError::Serialization)?;
    output.extend_from_slice(&bytes);
    Ok(())
}
