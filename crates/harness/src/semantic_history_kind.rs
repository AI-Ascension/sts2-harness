// SPDX-License-Identifier: MIT

//! The closed vocabulary of gameplay events this boundary stores, and what each kind must state.

use serde::{Deserialize, Serialize};

/// The closed vocabulary of gameplay events this boundary stores.
///
/// The set mirrors the game-mod companion's stated vocabulary. It is closed on purpose: a kind this
/// boundary does not know is refused rather than stored under a generic label, because a generic
/// label would make an unsupported event look captured.
///
/// Each kind also states the detail it must carry. The rules are per kind rather than global because
/// a detail that is missing where the host observed it is a refusal, not an event whose detail is
/// merely unknown: a consumer may never read an absent actor, target or content reference as "there
/// was none".
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticHistoryKind {
    /// A card was played.
    CardPlayed,
    /// Damage was dealt.
    Damage,
    /// Block was gained.
    Block,
    /// Health was restored.
    Heal,
    /// A character-specific resource changed.
    ResourceChanged,
    /// A status or power was applied.
    StatusApplied,
    /// A status or power was removed.
    StatusRemoved,
    /// A modifier was applied.
    ModifierApplied,
    /// A modifier was removed.
    ModifierRemoved,
    /// A card moved between piles.
    PileMoved,
    /// The party moved to another room or act.
    RoomTransitioned,
    /// A choice was made, including a reward or event decision.
    ChoiceMade,
    /// An offer was presented and is inspectable.
    OfferPresented,
    /// A purchase was made.
    PurchaseMade,
}

impl SemanticHistoryKind {
    /// Every kind this boundary stores, in a stable order.
    pub const ALL: [Self; 14] = [
        Self::CardPlayed,
        Self::Damage,
        Self::Block,
        Self::Heal,
        Self::ResourceChanged,
        Self::StatusApplied,
        Self::StatusRemoved,
        Self::ModifierApplied,
        Self::ModifierRemoved,
        Self::PileMoved,
        Self::RoomTransitioned,
        Self::ChoiceMade,
        Self::OfferPresented,
        Self::PurchaseMade,
    ];

    /// The stable lowercase name used in owner-defined text and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CardPlayed => "card_played",
            Self::Damage => "damage",
            Self::Block => "block",
            Self::Heal => "heal",
            Self::ResourceChanged => "resource_changed",
            Self::StatusApplied => "status_applied",
            Self::StatusRemoved => "status_removed",
            Self::ModifierApplied => "modifier_applied",
            Self::ModifierRemoved => "modifier_removed",
            Self::PileMoved => "pile_moved",
            Self::RoomTransitioned => "room_transitioned",
            Self::ChoiceMade => "choice_made",
            Self::OfferPresented => "offer_presented",
            Self::PurchaseMade => "purchase_made",
        }
    }

    /// Returns whether an event of this kind must name the actor that caused it.
    ///
    /// An effect that cannot name who caused it is not an authoritative event, so the actor is
    /// required everywhere the host observed one, and optional only where a room transition or an
    /// offer has no acting subject to name.
    #[must_use]
    pub const fn requires_actor(self) -> bool {
        matches!(
            self,
            Self::CardPlayed
                | Self::Damage
                | Self::Block
                | Self::Heal
                | Self::ResourceChanged
                | Self::StatusApplied
                | Self::StatusRemoved
                | Self::ModifierApplied
                | Self::ModifierRemoved
                | Self::PileMoved
                | Self::ChoiceMade
                | Self::PurchaseMade
        )
    }

    /// Returns whether an event of this kind must name the target it acted on.
    ///
    /// A targeted kind without a target is a refusal rather than an event whose target is unknown, so
    /// a consumer cannot read a missing target as "no target existed". A target supplied for a kind
    /// that does not act on one is refused for the same reason.
    #[must_use]
    pub const fn requires_target(self) -> bool {
        matches!(
            self,
            Self::Damage
                | Self::Block
                | Self::Heal
                | Self::StatusApplied
                | Self::StatusRemoved
                | Self::ModifierApplied
                | Self::ModifierRemoved
        )
    }

    /// Returns whether this kind may state a causal parent.
    ///
    /// Only an effect can have a cause. A room transition or an offer has no causal parent to state,
    /// so one supplied for such a kind is refused rather than recorded as unverified causality.
    #[must_use]
    pub const fn admits_cause(self) -> bool {
        matches!(
            self,
            Self::Damage
                | Self::Block
                | Self::Heal
                | Self::ResourceChanged
                | Self::StatusApplied
                | Self::StatusRemoved
                | Self::ModifierApplied
                | Self::ModifierRemoved
                | Self::PileMoved
                | Self::ChoiceMade
                | Self::PurchaseMade
        )
    }

    /// Returns whether an event of this kind must state a bounded quantity.
    ///
    /// A kind that reports how much happened without saying how much would be closed by a zeroed
    /// quantity, which is exactly the invented value this vocabulary refuses.
    #[must_use]
    pub const fn requires_quantity(self) -> bool {
        matches!(
            self,
            Self::Damage
                | Self::Block
                | Self::Heal
                | Self::ResourceChanged
                | Self::StatusApplied
                | Self::StatusRemoved
                | Self::ModifierApplied
                | Self::ModifierRemoved
        )
    }

    /// Returns whether an event of this kind must name a content reference.
    ///
    /// A card play, a pile move, a purchase and an offer all name the content they are about.
    /// Without that reference the event could not be resolved against a content manifest, so a
    /// consumer would have to guess which card or entity was meant.
    #[must_use]
    pub const fn requires_reference(self) -> bool {
        matches!(
            self,
            Self::CardPlayed | Self::PileMoved | Self::PurchaseMade | Self::OfferPresented
        )
    }
}
