// SPDX-License-Identifier: MIT

//! The closed inventories the history stores and queries by.
//!
//! These mirror the game-mod's `semantic_event_reference` vocabulary exactly. They are re-declared
//! here rather than imported because the harness has no compile-time dependency on the game-mod; the
//! names are the interface, and a producer name this history cannot state is refused at admission
//! rather than stored as a free-form string a consumer would have to interpret.

use serde::{Deserialize, Serialize};

/// One kind of authoritative gameplay event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticEventKind {
    /// A card was played, optionally naming its resolved target.
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

impl SemanticEventKind {
    /// Every kind this history can state, in a stable order.
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

    /// Resolves a producer name to a kind.
    ///
    /// A name outside the closed inventory is refused by the caller; it is never stored as an
    /// opaque label a consumer would have to interpret.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Returns whether an event of this kind may carry a causal parent.
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

    /// Returns whether an event of this kind must name a target subject.
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

    /// Returns whether an event of this kind must state a bounded quantity.
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

    /// Returns whether an event of this kind may name a target without requiring one.
    ///
    /// A card play, a choice, an offer and a purchase all have something they act on, but the
    /// boundary does not always observe it, so the target is admitted rather than required. A kind
    /// outside this set that carries a target is refused, so a target never appears on an event that
    /// cannot have one.
    #[must_use]
    pub const fn admits_target(self) -> bool {
        matches!(
            self,
            Self::CardPlayed | Self::ChoiceMade | Self::OfferPresented | Self::PurchaseMade
        ) || self.requires_target()
    }

    /// Returns whether an event of this kind must name a content identity.
    #[must_use]
    pub const fn requires_reference(self) -> bool {
        matches!(
            self,
            Self::CardPlayed | Self::PileMoved | Self::PurchaseMade | Self::OfferPresented
        )
    }

    /// Returns whether an event of this kind must name the actor that caused it.
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
}

/// The origin of one recorded event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticEventOrigin {
    /// Observed directly at the host boundary during this capture.
    Native,
    /// Computed by the owner from its own observed state, not observed as an event.
    Derived,
    /// Restored from saved history through an owned mod port.
    Imported,
}

impl SemanticEventOrigin {
    /// Every origin, in a stable order.
    pub const ALL: [Self; 3] = [Self::Native, Self::Derived, Self::Imported];

    /// The stable lowercase name used in owner-defined text and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Derived => "derived",
            Self::Imported => "imported",
        }
    }

    /// Resolves a producer name to an origin.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|origin| origin.name() == name)
    }

    /// Returns whether this origin may state an explicit causal parent.
    #[must_use]
    pub const fn admits_stated_parent(self) -> bool {
        matches!(self, Self::Native | Self::Derived)
    }
}
