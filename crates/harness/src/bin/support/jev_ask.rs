// SPDX-License-Identifier: MIT

//! Builds the questions one decision is asked as.
//!
//! A reward screen is two decisions the host splits across two screens: which reward, then which
//! card. Where the host has disclosed what a reward holds, both are asked in one call here, so the
//! reward is only opened when a card in it is wanted.
//!
//! This lives beside the record rather than inside it because the ask is the part that decides how
//! much a provider is told, and the record is the part that stores what came back. The card question
//! is the second consumer the request contract was waiting for, so it is admitted only where
//! something reads its answer; the profiles that own a reviewed question count keep the single
//! question they were reviewed with.

use serde_json::Value;
use sts2_harness::{
    ACTION_QUESTION, CARD_CHOICE_QUESTION, DisclosedCardChoice, SystemOneOption, SystemOneQuestion,
    action_instructions, build_choice_questions_request, card_choice_instructions,
    disclosed_card_choice,
};

type Failure = Box<dyn std::error::Error>;

/// Everything one prepared ask carries, in one value rather than a long argument list.
///
/// These travel together because they are decided together: the option identifiers, the catalog
/// they were drawn from and the advisory card set all describe one request.
pub(super) struct Ask {
    /// The request body to send.
    pub body: Value,
    /// The presented action identifiers, for the answer's containment check.
    pub ids: Vec<String>,
    /// The host catalog size the selection was drawn from.
    pub catalog_count: usize,
    /// Whether the evaluation profile prepares this body.
    pub tactical_enabled: bool,
    /// The disclosed card set, when one was asked beside the action.
    pub card_choice: Option<DisclosedCardChoice>,
}

/// What one ask is built from.
///
/// The objective and the hard constraints travel with the observation rather than beside it because
/// both are read out of the same bridge request, and a call that supplies one without the other
/// would be building a question from a decision nobody framed.
pub(super) struct AskContext<'a> {
    /// Provider model identifier.
    pub model: &'a str,
    /// The rendered observation the provider reads.
    pub state: &'a str,
    /// The presented action options.
    pub options: &'a [SystemOneOption],
    /// The host catalog size the selection was drawn from.
    pub catalog_count: usize,
    /// The observation the disclosure is read from.
    pub observation: &'a Value,
    /// The framing objective.
    pub objective: &'a str,
    /// The framing hard constraints.
    pub hard_constraints: &'a [String],
    /// Whether the evaluation profile is in use.
    pub tactical_enabled: bool,
    /// Whether more than one exchange is permitted.
    pub two_stage_allowed: bool,
}

/// Builds the question set for one decision.
///
/// The card a reward would hand over joins the action question whenever the host has disclosed one
/// pending choice. The evaluation profile is excluded because it owns its own question-count
/// contract (`1 + 7 * targets`), and the capture profile permits one transport invocation, so both
/// keep the single question they were reviewed with.
///
/// `two_stage_allowed` is the same flag that permits a second exchange at all, so the card question
/// is admitted on exactly the calls where more than one question could have been asked.
///
/// # Errors
///
/// Returns a [`Failure`] when the request builder refuses the question set. Nothing here repairs an
/// input it cannot ask about.
pub(super) fn build(context: &AskContext<'_>) -> Result<Ask, Failure> {
    let AskContext {
        model,
        state,
        options,
        catalog_count,
        observation,
        objective,
        hard_constraints,
        tactical_enabled,
        two_stage_allowed,
    } = *context;
    let card_choice = (!tactical_enabled && two_stage_allowed)
        .then(|| disclosed_card_choice(observation))
        .flatten();
    let action_instruction = action_instructions(objective, hard_constraints);
    let card_instruction = card_choice
        .as_ref()
        .map(|choice| card_choice_instructions(&choice.reward_id));
    let mut questions = vec![SystemOneQuestion::new(
        ACTION_QUESTION,
        options,
        &action_instruction,
    )];
    if let (Some(choice), Some(instructions)) = (&card_choice, &card_instruction) {
        questions.push(SystemOneQuestion::new(
            CARD_CHOICE_QUESTION,
            &choice.options,
            instructions,
        ));
    }
    Ok(Ask {
        body: build_choice_questions_request(model, state, &questions)?,
        ids: options.iter().map(|option| option.id.clone()).collect(),
        catalog_count,
        tactical_enabled,
        card_choice,
    })
}
