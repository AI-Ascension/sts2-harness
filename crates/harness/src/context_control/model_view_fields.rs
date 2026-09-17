// SPDX-License-Identifier: MIT

//! The closed field table of a model-view projection.
//!
//! This is the whole vocabulary. Every entry is a compiled constant, so the set of fields a recipe
//! may name is fixed at build time and cannot be extended by a configuration file or a request.
//!
//! Two entries are narrower than the fair-play floor on purpose, because a model view is strictly
//! smaller than what may leave the host:
//!
//! - `legal_actions` is owner-only — the catalog reaches a model through its own reviewed channel,
//!   and
//! - `player.deck`, `player.discard`, and `player.exhaust` are owner-only — their contents and order
//!   encode draws this invocation has not been shown.

use super::model_view_catalog::{
    ElementKind, FieldPresence, FieldProtection, FieldShape, FieldSpec, FieldType, ViewContext,
};

const fn required(name: &'static str, shape: FieldShape) -> FieldSpec {
    FieldSpec {
        context: ViewContext::Root,
        name,
        shape,
        presence: FieldPresence::Required,
        nullable: false,
        protection: FieldProtection::ModelVisible,
    }
}

const fn optional(name: &'static str, shape: FieldShape) -> FieldSpec {
    FieldSpec {
        context: ViewContext::Root,
        name,
        shape,
        presence: FieldPresence::Optional,
        nullable: false,
        protection: FieldProtection::ModelVisible,
    }
}

const fn nullable(mut spec: FieldSpec) -> FieldSpec {
    spec.nullable = true;
    spec
}

const fn owner_only(mut spec: FieldSpec) -> FieldSpec {
    spec.protection = FieldProtection::OwnerOnly;
    spec
}

const fn in_context(mut spec: FieldSpec, context: ViewContext) -> FieldSpec {
    spec.context = context;
    spec
}

const fn scalar(kind: FieldType) -> FieldShape {
    FieldShape::Scalar(kind)
}

const fn object(context: ViewContext) -> FieldShape {
    FieldShape::Object(context)
}

const fn cards() -> FieldShape {
    FieldShape::Collection(ElementKind::Object(ViewContext::Card), 256)
}

const fn identities() -> FieldShape {
    FieldShape::Collection(ElementKind::Identity, 256)
}

/// The complete, closed field vocabulary of a model-view projection.
pub const FIELD_CATALOG: &[FieldSpec] = &[
    // Observation root
    required("state_id", scalar(FieldType::Identity)),
    required("generation", scalar(FieldType::Number)),
    nullable(optional("visible_seed", scalar(FieldType::Text))),
    required("player", object(ViewContext::Player)),
    required("state", object(ViewContext::State)),
    owner_only(required(
        "legal_actions",
        FieldShape::Collection(ElementKind::Object(ViewContext::LegalAction), 256),
    )),
    // Player
    in_context(
        required("hp", scalar(FieldType::Number)),
        ViewContext::Player,
    ),
    in_context(
        required("max_hp", scalar(FieldType::Number)),
        ViewContext::Player,
    ),
    in_context(
        required("energy", scalar(FieldType::Number)),
        ViewContext::Player,
    ),
    in_context(
        required("gold", scalar(FieldType::Number)),
        ViewContext::Player,
    ),
    in_context(required("hand", cards()), ViewContext::Player),
    in_context(owner_only(required("deck", cards())), ViewContext::Player),
    in_context(
        owner_only(required("discard", cards())),
        ViewContext::Player,
    ),
    in_context(
        owner_only(required("exhaust", cards())),
        ViewContext::Player,
    ),
    // Card
    in_context(
        required("card_id", scalar(FieldType::Identity)),
        ViewContext::Card,
    ),
    in_context(required("name", scalar(FieldType::Text)), ViewContext::Card),
    in_context(
        required("cost", scalar(FieldType::Number)),
        ViewContext::Card,
    ),
    in_context(
        required("upgraded", scalar(FieldType::Boolean)),
        ViewContext::Card,
    ),
    // Enemy
    in_context(
        required("enemy_id", scalar(FieldType::Identity)),
        ViewContext::Enemy,
    ),
    in_context(
        required("name", scalar(FieldType::Text)),
        ViewContext::Enemy,
    ),
    in_context(
        required("hp", scalar(FieldType::Number)),
        ViewContext::Enemy,
    ),
    in_context(
        required("max_hp", scalar(FieldType::Number)),
        ViewContext::Enemy,
    ),
    in_context(
        required("intent", object(ViewContext::Intent)),
        ViewContext::Enemy,
    ),
    // Intent
    in_context(
        required("kind", scalar(FieldType::Identity)),
        ViewContext::Intent,
    ),
    in_context(
        nullable(optional("damage", scalar(FieldType::Number))),
        ViewContext::Intent,
    ),
    in_context(
        nullable(optional("hits", scalar(FieldType::Number))),
        ViewContext::Intent,
    ),
    // State
    in_context(
        required("state", scalar(FieldType::Identity)),
        ViewContext::State,
    ),
    in_context(optional("characters", identities()), ViewContext::State),
    in_context(
        nullable(optional("node_id", scalar(FieldType::Identity))),
        ViewContext::State,
    ),
    in_context(optional("options", identities()), ViewContext::State),
    in_context(
        optional("turn_index", scalar(FieldType::Number)),
        ViewContext::State,
    ),
    in_context(
        optional(
            "enemies",
            FieldShape::Collection(ElementKind::Object(ViewContext::Enemy), 64),
        ),
        ViewContext::State,
    ),
    in_context(optional("choices", identities()), ViewContext::State),
    in_context(
        optional(
            "items",
            FieldShape::Collection(ElementKind::Object(ViewContext::ShopItem), 128),
        ),
        ViewContext::State,
    ),
    in_context(
        nullable(optional("reason", scalar(FieldType::Text))),
        ViewContext::State,
    ),
    in_context(
        optional("code", scalar(FieldType::Identity)),
        ViewContext::State,
    ),
    // ShopItem
    in_context(
        required("item_id", scalar(FieldType::Identity)),
        ViewContext::ShopItem,
    ),
    in_context(
        required("name", scalar(FieldType::Text)),
        ViewContext::ShopItem,
    ),
    in_context(
        required("price", scalar(FieldType::Number)),
        ViewContext::ShopItem,
    ),
    // LegalAction
    in_context(
        required("action_id", scalar(FieldType::Identity)),
        ViewContext::LegalAction,
    ),
    in_context(
        required("action", object(ViewContext::Action)),
        ViewContext::LegalAction,
    ),
    // Action
    in_context(
        required("kind", scalar(FieldType::Identity)),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("character_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("node_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("card_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("player_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("target_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("reward_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("item_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
    in_context(
        nullable(optional("choice_id", scalar(FieldType::Identity))),
        ViewContext::Action,
    ),
];
