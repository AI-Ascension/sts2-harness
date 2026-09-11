// SPDX-License-Identifier: MIT

use super::diagnostic::{Diagnostic, DiagnosticCode, StructuralLocation};
use serde::de::DeserializeOwned;

pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_DEPTH: usize = 32;
pub const MAX_COLLECTION_ITEMS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderLimits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_string_bytes: usize,
    pub max_array_items: usize,
    pub max_object_members: usize,
}

impl Default for DecoderLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_SOURCE_BYTES,
            max_depth: MAX_DEPTH,
            max_string_bytes: super::ids::MAX_STRING_BYTES,
            max_array_items: MAX_COLLECTION_ITEMS,
            max_object_members: MAX_COLLECTION_ITEMS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Limit(Diagnostic),
    DuplicateKey(Diagnostic),
    Syntax(Diagnostic),
    Schema(Diagnostic),
}

impl DecodeError {
    pub fn diagnostic(&self) -> &Diagnostic {
        match self {
            Self::Limit(value)
            | Self::DuplicateKey(value)
            | Self::Syntax(value)
            | Self::Schema(value) => value,
        }
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "workflow decode failed: {:?}",
            self.diagnostic().code
        )
    }
}

impl std::error::Error for DecodeError {}

pub fn decode_strict<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    decode_strict_with_limits(bytes, DecoderLimits::default())
}

pub fn decode_strict_with_limits<T: DeserializeOwned>(
    bytes: &[u8],
    limits: DecoderLimits,
) -> Result<T, DecodeError> {
    if bytes.len() > limits.max_bytes {
        return Err(limit(DiagnosticCode::CollectionTooLarge, "$", 0));
    }
    super::scanner::scan(bytes, limits)?;
    serde_json::from_slice(bytes).map_err(|error| schema_error(error.classify(), error.to_string()))
}

pub fn decode_json(bytes: &[u8]) -> Result<serde_json::Value, DecodeError> {
    decode_strict(bytes)
}

fn limit(code: DiagnosticCode, path: &str, offset: usize) -> DecodeError {
    DecodeError::Limit(Diagnostic::error(code, location(path, offset)))
}

fn schema_error(class: serde_json::error::Category, message: String) -> DecodeError {
    let code = if message.contains("unknown field") {
        DiagnosticCode::UnknownField
    } else if message.contains("missing field") {
        DiagnosticCode::MissingField
    } else if message.contains("unknown variant") {
        DiagnosticCode::UnknownEnum
    } else if message.contains("invalid type") {
        DiagnosticCode::TypeMismatch
    } else {
        match class {
            serde_json::error::Category::Io => DiagnosticCode::InvalidJson,
            serde_json::error::Category::Syntax => DiagnosticCode::InvalidJson,
            serde_json::error::Category::Data => DiagnosticCode::TypeError,
            serde_json::error::Category::Eof => DiagnosticCode::InvalidJson,
        }
    };
    DecodeError::Schema(Diagnostic::error(code, StructuralLocation::root()))
}

fn location(path: &str, offset: usize) -> StructuralLocation {
    StructuralLocation {
        path: path.to_owned(),
        offset: u64::try_from(offset).ok(),
    }
}
