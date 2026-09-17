// SPDX-License-Identifier: MIT

//! The closed field vocabulary a model-view projection may select (issue #110).
//!
//! The fair-play validator in `exo::sandbox` decides whether an observation may leave the host at
//! all. This catalog decides which *already admitted* fields may become model-visible, and it does
//! so from a static table rather than from a query language. There is deliberately no JSONPath, no
//! wildcard, no expression evaluation, and no caller-supplied pointer arithmetic: a declared view
//! path is resolved hop by hop against [`FIELD_CATALOG`], so an unknown or protected segment is a
//! typed refusal rather than a silent miss.
//!
//! The table mirrors the structural schema the fair-play validator already enforces. Two deliberate
//! restrictions narrow it further, because a model view is a strictly smaller set than the fair-play
//! floor:
//!
//! - the owner's legal-action catalog is [`FieldProtection::OwnerOnly`] — it reaches a model through
//!   its own reviewed channel and is never a projection target, and
//! - the unseen card piles (`player.deck`, `player.discard`, `player.exhaust`) are
//!   [`FieldProtection::OwnerOnly`] because their contents and order encode draws this invocation
//!   has not been shown.

use serde::{Deserialize, Serialize};

/// Upper bound on the model-visible elements of one projected collection.
pub const MAX_PROJECTED_COLLECTION: usize = 256;
/// Upper bound on the declared paths of one recipe.
pub const MAX_MODEL_VIEW_FIELDS: usize = 64;
/// Upper bound on the segments of one declared path.
pub const MAX_MODEL_VIEW_PATH_SEGMENTS: usize = 4;

/// The structural context a field name is resolved against.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewContext {
    Root,
    Player,
    Card,
    Enemy,
    Intent,
    State,
    ShopItem,
    LegalAction,
    Action,
}

impl ViewContext {
    /// Stable name used in metadata and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "observation",
            Self::Player => "player",
            Self::Card => "card",
            Self::Enemy => "enemy",
            Self::Intent => "intent",
            Self::State => "state",
            Self::ShopItem => "shop_item",
            Self::LegalAction => "legal_action",
            Self::Action => "action",
        }
    }
}

/// Scalar field value types a projection may emit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldType {
    Identity,
    Text,
    Number,
    Boolean,
}

impl FieldType {
    /// Stable name used in metadata and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Text => "text",
            Self::Number => "number",
            Self::Boolean => "boolean",
        }
    }
}

/// The element type of a declared collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElementKind {
    Identity,
    Object(ViewContext),
}

/// The value shape a declared field may have.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldShape {
    Scalar(FieldType),
    Object(ViewContext),
    Collection(ElementKind, usize),
}

impl FieldShape {
    /// Stable name used in metadata and diagnostics.
    #[must_use]
    pub const fn kind_str(self) -> &'static str {
        match self {
            Self::Scalar(FieldType::Identity) => "identity",
            Self::Scalar(FieldType::Text) => "text",
            Self::Scalar(FieldType::Number) => "number",
            Self::Scalar(FieldType::Boolean) => "boolean",
            Self::Object(_) => "object",
            Self::Collection(_, _) => "collection",
        }
    }

    /// The declared collection bound, when this shape is a collection.
    #[must_use]
    pub const fn collection_bound(self) -> Option<usize> {
        match self {
            Self::Collection(_, bound) => Some(bound),
            Self::Scalar(_) | Self::Object(_) => None,
        }
    }
}

/// Whether the source schema guarantees the field, or the recipe must tolerate its absence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldPresence {
    Required,
    Optional,
}

/// Whether this field may ever appear in model-visible output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldProtection {
    ModelVisible,
    OwnerOnly,
}

/// One resolved field of the model-view vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FieldSpec {
    pub context: ViewContext,
    pub name: &'static str,
    pub shape: FieldShape,
    pub presence: FieldPresence,
    pub nullable: bool,
    pub protection: FieldProtection,
}

pub use super::model_view_fields::FIELD_CATALOG;

/// Resolves one field name inside a structural context, or `None` for an unknown segment.
#[must_use]
pub fn spec(context: ViewContext, name: &str) -> Option<&'static FieldSpec> {
    FIELD_CATALOG
        .iter()
        .find(|entry| entry.context == context && entry.name == name)
}

/// Every field name the catalog declares for one context.
#[must_use]
pub fn declared_names(context: ViewContext) -> Vec<&'static str> {
    FIELD_CATALOG
        .iter()
        .filter(|entry| entry.context == context)
        .map(|entry| entry.name)
        .collect()
}

/// One catalog entry rendered for owner/consumer metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFieldMetadata {
    /// The owner context this field is a member of.
    pub context: String,
    /// The field name as it appears in the source object.
    pub name: String,
    /// Scalar type, `object`, or `collection`.
    pub field_type: String,
    /// `required` or `optional`.
    pub presence: String,
    /// Whether an explicit `null` is a legal value.
    pub nullable: bool,
    /// `model_visible` or `owner_only`.
    pub protection: String,
    /// The declared element bound when this field is a collection.
    pub collection_bound: Option<usize>,
}

impl ModelFieldMetadata {
    fn from_spec(entry: &FieldSpec) -> Self {
        Self {
            context: entry.context.as_str().to_owned(),
            name: entry.name.to_owned(),
            field_type: entry.shape.kind_str().to_owned(),
            presence: match entry.presence {
                FieldPresence::Required => "required",
                FieldPresence::Optional => "optional",
            }
            .to_owned(),
            nullable: entry.nullable,
            protection: match entry.protection {
                FieldProtection::ModelVisible => "model_visible",
                FieldProtection::OwnerOnly => "owner_only",
            }
            .to_owned(),
            collection_bound: entry.shape.collection_bound(),
        }
    }
}

/// The full catalog as owner/consumer metadata, in declaration order.
///
/// A design surface can render the permitted field picker from this list without any projection
/// implementation being reachable, and every entry names its exact type and protection.
#[must_use]
pub fn catalog_metadata() -> Vec<ModelFieldMetadata> {
    FIELD_CATALOG
        .iter()
        .map(ModelFieldMetadata::from_spec)
        .collect()
}
