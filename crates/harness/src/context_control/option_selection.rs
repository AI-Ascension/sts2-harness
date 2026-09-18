// SPDX-License-Identifier: MIT

//! Chooses which legal actions are presented to a provider, and records what was held back.
//!
//! The host-generated catalog is bounded at 256 entries and is authoritative for legality. Handing
//! all of it to a provider is expensive in the shared state-and-question token budget and, for a
//! provider whose confidence is computed from the shape of a probability distribution, actively
//! misleading: twenty near-duplicate options split the mass between strategically identical choices
//! and return a low confidence that reads as "unsure what to do" when it means "unsure which of
//! three identical things to name".
//!
//! This module therefore decides what is *askable*. It never decides what is good: the presented
//! order is the catalog order, and nothing here ranks, scores, or prefers an action.
//!
//! Narrowing may only ever hide an option the host listed, never invent one, so every rule is
//! conservative and every removal is recorded with a reason that names the option it folded into.
//! Two safety properties hold by construction. The first occurrence of any signature is always the
//! presented one, so a turn-ending action in the catalog is always presented and the provider keeps
//! an escape; folding can remove a second copy of it, never the option itself. And a selection that
//! would leave fewer than two options presents the full catalog instead, because one option is not
//! a question. A rule that withholds a whole kind would break the first property, so any such rule
//! has to restore the turn-ending action explicitly; none exists here today.
//!
//! One rule that a first reading suggests is deliberately absent. Withholding an unaffordable card
//! would contradict the host, which lists an action only when it is legal; an affordability filter
//! could therefore only ever remove a play the host said was available. Affordability belongs
//! in the derived-exact facts beside the state, where it informs the provider, not in the
//! option set, where it would overrule the authority.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable schema identity of this selection record.
pub const OPTION_SELECTION_SCHEMA: &str = "ascension.context-control.option-selection.v1";

/// Number of presented options above which a single question is split into two stages.
pub const MAX_PRESENTED_OPTIONS: usize = 24;

/// Declared bound on the legal-action collection, mirroring the model-view vocabulary.
const MAX_CATALOG: usize = 256;

/// How the presented options should be asked about.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    /// Exactly one action is legal. No provider call is needed and none should be made.
    Forced,
    /// The presented options fit in one question.
    Single,
    /// The presented options exceed the bound; ask for a kind first, then an action within it.
    TwoStage,
}

/// One option a provider may be asked to choose.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PresentedOption {
    /// The concrete, legal action identifier this option resolves to.
    pub action_id: String,
    /// The action kind, used as the class in a two-stage question.
    pub kind: String,
    /// Other catalog identifiers this option stands for, in catalog order.
    pub folded: Vec<String>,
}

/// Why an option was not presented.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum WithheldReason {
    /// Strategically identical to a presented option under the admitted vocabulary.
    Duplicate {
        /// The presented option this one folded into.
        folded_into: String,
    },
    /// The entry did not carry a usable identifier or action object.
    Malformed,
}

/// One catalog entry that was not presented, with the reason.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WithheldOption {
    /// The catalog identifier that was held back, when it had one.
    pub action_id: String,
    /// Why it was held back.
    #[serde(flatten)]
    pub reason: WithheldReason,
}

/// The option set a provider is asked about, and everything that was held back.
///
/// `presented` and `withheld` partition the catalog: every entry appears in exactly one of them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionSelection {
    /// How to ask about the presented options.
    pub mode: SelectionMode,
    /// The options to present, in catalog order.
    pub presented: Vec<PresentedOption>,
    /// The entries held back, each naming its reason.
    pub withheld: Vec<WithheldOption>,
}

impl OptionSelection {
    /// Selects the options to present from an admitted observation's legal-action catalog.
    ///
    /// `bound` is the number of options above which the selection asks for a two-stage question.
    #[must_use]
    pub fn from_observation(observation: &Value, bound: usize) -> Self {
        let catalog = observation
            .get("legal_actions")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let entries = read_entries(catalog);
        let full: Vec<PresentedOption> = entries
            .iter()
            .map(|entry| PresentedOption {
                action_id: entry.action_id.to_owned(),
                kind: entry.kind.to_owned(),
                folded: Vec::new(),
            })
            .collect();
        let malformed: Vec<WithheldOption> = catalog
            .iter()
            .take(MAX_CATALOG)
            .filter(|item| read_entry(item).is_none())
            .map(|item| WithheldOption {
                action_id: item
                    .get("action_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                reason: WithheldReason::Malformed,
            })
            .collect();

        if full.len() == 1 {
            return Self {
                mode: SelectionMode::Forced,
                presented: full,
                withheld: malformed,
            };
        }
        let (presented, mut withheld) = collapse(&entries);
        // Fewer than two askable options is not a question; present the catalog unchanged.
        let (presented, withheld) = if presented.len() < 2 {
            (full, malformed)
        } else {
            withheld.extend(malformed.iter().cloned());
            (presented, withheld)
        };
        let mode = if presented.len() > bound {
            SelectionMode::TwoStage
        } else {
            SelectionMode::Single
        };
        Self {
            mode,
            presented,
            withheld,
        }
    }

    /// The distinct action kinds of the presented options, in first-seen order.
    #[must_use]
    pub fn classes(&self) -> Vec<String> {
        let mut classes: Vec<String> = Vec::new();
        for option in &self.presented {
            if !classes.iter().any(|kind| kind == &option.kind) {
                classes.push(option.kind.clone());
            }
        }
        classes
    }
}

/// One readable catalog entry.
struct Entry<'a> {
    action_id: &'a str,
    kind: &'a str,
    signature: String,
}

/// Reads every usable catalog entry, in catalog order.
fn read_entries(catalog: &[Value]) -> Vec<Entry<'_>> {
    catalog
        .iter()
        .take(MAX_CATALOG)
        .filter_map(read_entry)
        .collect()
}

/// Reads one catalog entry, refusing an entry without an identifier or an action object.
fn read_entry(item: &Value) -> Option<Entry<'_>> {
    let action_id = item.get("action_id").and_then(Value::as_str)?;
    let action = item.get("action")?.as_object()?;
    let kind = action.get("kind").and_then(Value::as_str)?;
    // Everything the vocabulary lets an action carry except the card instance identity, which is
    // what distinguishes two copies of the same card in hand.
    let mut signature = String::from(kind);
    for field in [
        "character_id",
        "node_id",
        "player_id",
        "target_id",
        "reward_id",
        "item_id",
        "choice_id",
    ] {
        signature.push('|');
        signature.push_str(
            action
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
    }
    Some(Entry {
        action_id,
        kind,
        signature,
    })
}

/// Folds entries that are identical under the admitted vocabulary into their first occurrence.
///
/// Card instance identity is deliberately not part of the signature: two copies of one card in
/// hand, aimed at the same target, are the same question asked twice. The first occurrence in
/// catalog order is the representative, so the result is stable for a fixed input.
fn collapse(entries: &[Entry<'_>]) -> (Vec<PresentedOption>, Vec<WithheldOption>) {
    let mut presented: Vec<PresentedOption> = Vec::new();
    let mut signatures: Vec<&str> = Vec::new();
    let mut withheld = Vec::new();
    for entry in entries {
        let seen = signatures
            .iter()
            .position(|signature| *signature == entry.signature.as_str());
        match seen.and_then(|index| presented.get_mut(index)) {
            Some(option) => {
                option.folded.push(entry.action_id.to_owned());
                withheld.push(WithheldOption {
                    action_id: entry.action_id.to_owned(),
                    reason: WithheldReason::Duplicate {
                        folded_into: option.action_id.clone(),
                    },
                });
            }
            None => {
                signatures.push(entry.signature.as_str());
                presented.push(PresentedOption {
                    action_id: entry.action_id.to_owned(),
                    kind: entry.kind.to_owned(),
                    folded: Vec::new(),
                });
            }
        }
    }
    (presented, withheld)
}

#[cfg(test)]
#[path = "option_selection_tests.rs"]
mod tests;
