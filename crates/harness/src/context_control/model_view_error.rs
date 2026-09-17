// SPDX-License-Identifier: MIT

//! Recipe construction, resolution, and projection failures.

use std::fmt::{Display, Formatter};

/// Recipe construction and resolution failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelViewProjectionError {
    /// The schema, selector identity, or revision is malformed.
    InvalidInput,
    /// No field was declared.
    EmptyPath,
    /// The recipe declares more fields than the bound permits.
    TooManyFields { bound: usize },
    /// One declared path has more segments than the bound permits.
    PathTooLong { bound: usize },
    /// A named segment was empty.
    EmptySegment,
    /// A named segment is not part of the closed catalog for its context.
    UnknownPath { path: String },
    /// A named segment is owner-only and must never enter model-visible bytes.
    ProtectedPath { path: String },
    /// A scalar or identity collection was treated as a container.
    NotAnObject { path: String },
    /// A nested object was named without selecting any member.
    UnresolvedObject { path: String },
    /// An object-element collection was named without its explicit index and member.
    MissingIndex { path: String },
    /// Two declared paths resolve to the same output path.
    DuplicateOutput { path: String },
    /// A required model-visible root field was abandoned by this recipe.
    RequiredFieldOmitted { field: String },
    /// A recipe claimed an identity already bound to a different revision.
    RepeatedSelector { selector_id: String },
    /// A request field is protected by its source and may not be projected.
    ProtectedRequestField { path: String },
    /// The admitted source failed its fair-play revalidation immediately before projection.
    SourceInvalid,
    /// The source holds a value whose shape contradicts the declared field shape.
    SourceShapeMismatch { path: String },
    /// A required source field is absent from the admitted observation.
    MissingSourceField { path: String },
    /// A source collection is larger than the declared bound.
    CollectionBoundExceeded { path: String, bound: usize },
    /// The projected bytes exceed the model-view bound.
    OversizedOutput { bound: usize },
    /// An excluded sentinel field is present in the projected bytes.
    ExcludedSentinelPresent { path: String },
    /// A prepared projection was approved against a different source or recipe revision.
    ApprovalFenced,
    /// The recipe could not be canonically encoded.
    Encode,
}

impl Display for ModelViewProjectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput => formatter
                .write_str("model-view projection schema, selector, or revision is invalid"),
            Self::EmptyPath => formatter.write_str("model-view projection declares no field"),
            Self::TooManyFields { bound } => write!(
                formatter,
                "model-view projection exceeds the bound of {bound} fields"
            ),
            Self::PathTooLong { bound } => write!(
                formatter,
                "model-view projection path exceeds the bound of {bound} segments"
            ),
            Self::EmptySegment => formatter.write_str("model-view projection path is empty"),
            Self::UnknownPath { path } => {
                write!(
                    formatter,
                    "model-view projection field {path} is not in the catalog"
                )
            }
            Self::ProtectedPath { path } => write!(
                formatter,
                "model-view projection field {path} is owner-only"
            ),
            Self::NotAnObject { path } => {
                write!(
                    formatter,
                    "model-view projection field {path} is not a container"
                )
            }
            Self::UnresolvedObject { path } => write!(
                formatter,
                "model-view projection field {path} selects a container without a member"
            ),
            Self::MissingIndex { path } => write!(
                formatter,
                "model-view projection collection {path} needs an explicit index and member"
            ),
            Self::DuplicateOutput { path } => {
                write!(
                    formatter,
                    "model-view projection output {path} is declared twice"
                )
            }
            Self::RequiredFieldOmitted { field } => write!(
                formatter,
                "model-view projection omits required field {field}"
            ),
            Self::RepeatedSelector { selector_id } => write!(
                formatter,
                "model-view selector {selector_id} is already bound to a different revision"
            ),
            Self::ProtectedRequestField { path } => write!(
                formatter,
                "request field {path} is protected and cannot be projected"
            ),
            Self::SourceInvalid => {
                formatter.write_str("model-view source failed fair-play validation")
            }
            Self::SourceShapeMismatch { path } => write!(
                formatter,
                "model-view source field {path} contradicts its declared shape"
            ),
            Self::MissingSourceField { path } => {
                write!(formatter, "model-view source field {path} is absent")
            }
            Self::CollectionBoundExceeded { path, bound } => write!(
                formatter,
                "model-view source collection {path} exceeds the bound of {bound}"
            ),
            Self::OversizedOutput { bound } => write!(
                formatter,
                "model-view projection exceeds the bound of {bound} bytes"
            ),
            Self::ExcludedSentinelPresent { path } => write!(
                formatter,
                "model-view projection contains excluded field {path}"
            ),
            Self::ApprovalFenced => {
                formatter.write_str("model-view approval is fenced by a changed source or recipe")
            }
            Self::Encode => formatter.write_str("model-view projection encoding failed"),
        }
    }
}

impl std::error::Error for ModelViewProjectionError {}

impl ModelViewProjectionError {
    /// A stable, precise reason code for this refusal.
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "model_view_projection_invalid_input",
            Self::EmptyPath => "model_view_projection_empty_path",
            Self::TooManyFields { .. } => "model_view_projection_too_many_fields",
            Self::PathTooLong { .. } => "model_view_projection_path_too_long",
            Self::EmptySegment => "model_view_projection_empty_segment",
            Self::UnknownPath { .. } => "model_view_projection_unknown_path",
            Self::ProtectedPath { .. } => "model_view_projection_protected_path",
            Self::NotAnObject { .. } => "model_view_projection_not_an_object",
            Self::UnresolvedObject { .. } => "model_view_projection_unresolved_object",
            Self::MissingIndex { .. } => "model_view_projection_missing_index",
            Self::DuplicateOutput { .. } => "model_view_projection_duplicate_output",
            Self::RequiredFieldOmitted { .. } => "model_view_projection_required_field_omitted",
            Self::RepeatedSelector { .. } => "model_view_projection_repeated_selector",
            Self::ProtectedRequestField { .. } => "model_view_projection_protected_request_field",
            Self::SourceInvalid => "model_view_projection_source_invalid",
            Self::SourceShapeMismatch { .. } => "model_view_projection_source_shape_mismatch",
            Self::MissingSourceField { .. } => "model_view_projection_missing_source_field",
            Self::CollectionBoundExceeded { .. } => {
                "model_view_projection_collection_bound_exceeded"
            }
            Self::OversizedOutput { .. } => "model_view_projection_oversized_output",
            Self::ExcludedSentinelPresent { .. } => {
                "model_view_projection_excluded_sentinel_present"
            }
            Self::ApprovalFenced => "model_view_projection_approval_fenced",
            Self::Encode => "model_view_projection_encode",
        }
    }
}
