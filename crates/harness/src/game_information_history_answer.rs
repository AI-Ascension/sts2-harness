// SPDX-License-Identifier: MIT

//! Serving one bounded history question, and projecting the answer a caller can act on.
//!
//! The answer is the port's own answer rather than a re-projection of it: a page carries the events
//! the store holds, the gaps the window declares and the continuation that resumes it, so a caller
//! that receives a short page can tell a quiet run from an incomplete capture.

use super::*;
use crate::semantic_history::{
    SemanticHistoryAgentPort, SemanticHistoryAuthority, SemanticHistoryBinding,
    SemanticHistoryContinuation, SemanticHistoryExplanation, SemanticHistoryPage,
    SemanticHistoryStore, SemanticHistorySummary,
};
use serde_json::json;

impl LookupSession {
    /// Attaches the durable history this owner keeps for this session's scope.
    ///
    /// Attachment is the owner's admission and is checked twice: here, and again on every read. A
    /// store serving another run, profile or epoch is refused rather than consulted, and a session
    /// that already holds history refuses a second store instead of quietly replacing the first.
    ///
    /// The session scope has no episode axis of its own: an episode is carried per event, so a page
    /// can filter by episode without the store being partitioned by it.
    pub fn attach_history(&mut self, store: SemanticHistoryStore) -> Result<(), LookupError> {
        if self.history.is_some() {
            return Err(LookupError::Scope);
        }
        SemanticHistoryAgentPort::grant(
            &store,
            SemanticHistoryAuthority::HarnessOwned,
            expected(&self.binding),
        )
        .map_err(refuse)?;
        self.history = Some(store);
        Ok(())
    }
}

/// Serves one history turn through the store the owner attached to this session.
///
/// A session the owner attached no history to has no history tool at all, so that is the answer
/// before the question is even read: a refusal with a capability name rather than an empty history.
pub(crate) fn serve_history(session: &LookupSession, turn: LookupTurn) -> LookupFeedback {
    let LookupTurn::History {
        operation_id,
        request,
    } = turn
    else {
        return LookupFeedback::Error(LookupError::Invalid);
    };
    let Some(store) = session.history.as_ref() else {
        return LookupFeedback::Error(LookupError::MissingCapability);
    };
    let ask = match decode(&request) {
        Ok(ask) => ask,
        Err(error) => return LookupFeedback::Error(error),
    };
    match read(store, &session.binding, &ask) {
        // The operation identity is echoed so a caller can match a bounded answer to the question it
        // asked, without having to guess which of its outstanding reads this one answers.
        Ok(answer) => LookupFeedback::History {
            operation_id,
            answer,
        },
        Err(error) => LookupFeedback::Error(error),
    }
}

/// Decodes the canonical request a turn carries, refusing a profile this boundary did not write.
fn decode(request: &[u8]) -> Result<HistoryAsk, LookupError> {
    let value = crate::game_information_validation::decode_strict(request)?;
    let request: HistoryRequest =
        serde_json::from_value(value).map_err(|_| LookupError::Invalid)?;
    if request.profile != HISTORY_AGENT_PROFILE {
        return Err(LookupError::Invalid);
    }
    Ok(request.ask)
}

/// Answers one decoded question through a port granted for this session's own owner scope.
///
/// The grant is taken against the session rather than against the store's copy of its own scope: a
/// store that no longer serves this run, profile or epoch is refused here as well as at attach time,
/// so a session that outlived an owner change cannot keep reading through a stale attachment.
fn read(
    store: &SemanticHistoryStore,
    binding: &LookupBinding,
    ask: &HistoryAsk,
) -> Result<Value, LookupError> {
    let port = SemanticHistoryAgentPort::grant(
        store,
        SemanticHistoryAuthority::HarnessOwned,
        expected(binding),
    )
    .map_err(refuse)?;
    match ask {
        HistoryAsk::Page {
            query,
            continuation,
        } => {
            let continuation = continuation
                .as_ref()
                .map(|cursor| SemanticHistoryContinuation {
                    cursor: cursor.cursor(),
                });
            let page = port
                .page_from(query, continuation.as_ref())
                .map_err(refuse)?;
            page_value(&page)
        }
        HistoryAsk::Summary { branch_id } => {
            Ok(summary_value(&port.summary(branch_id).map_err(refuse)?))
        }
        HistoryAsk::Explain {
            branch_id,
            event_id,
            limits,
        } => explanation_value(
            &port
                .explain(branch_id, event_id, limits.bounds())
                .map_err(refuse)?,
        ),
    }
}

/// The owner scope a session and a store must agree on before history is served.
fn expected(binding: &LookupBinding) -> SemanticHistoryBinding {
    SemanticHistoryBinding {
        project_id: binding.scope.project_id.clone(),
        run_id: binding.scope.run_id.clone(),
        agent_id: binding.scope.agent_id.clone(),
        game_profile: binding.game_profile.clone(),
        content_manifest_id: binding.content_manifest_id.clone(),
        locale: binding.locale.clone(),
        authority_epoch: binding.authority_epoch,
    }
}

fn page_value(page: &SemanticHistoryPage) -> Result<Value, LookupError> {
    Ok(json!({
        "authority": HISTORY_AUTHORITY,
        "events": serde_json::to_value(&page.events).map_err(|_| LookupError::Invalid)?,
        "continuation": page
            .continuation
            .as_ref()
            .map(|continuation| WireCursor::from_cursor(&continuation.cursor)),
        "gaps": page.gaps.iter().map(|status| status.name()).collect::<Vec<_>>(),
        "before_capture": page.before_capture,
    }))
}

fn summary_value(summary: &SemanticHistorySummary) -> Value {
    json!({
        "authority": HISTORY_AUTHORITY,
        "branch_id": summary.branch_id,
        "total": summary.total,
        "captured": summary.captured,
        "gaps": summary.gaps,
        "first_sequence": summary.first_sequence,
        "last_sequence": summary.last_sequence,
    })
}

fn explanation_value(explanation: &SemanticHistoryExplanation) -> Result<Value, LookupError> {
    let traversal = &explanation.traversal;
    Ok(json!({
        "authority": HISTORY_AUTHORITY,
        "event_id": explanation.event_id,
        "kind": explanation.kind,
        "value": serde_json::to_value(&explanation.value).map_err(|_| LookupError::Invalid)?,
        "traversal": {
            "root_event_id": traversal.root_event_id,
            "links": traversal.links.iter().map(|link| json!({
                "from_event_id": link.from_event_id,
                "to_event_id": link.to_event_id,
                "depth": link.depth,
            })).collect::<Vec<_>>(),
            "visited": traversal.visited,
            "truncated": traversal.truncated,
            "root_cause_unstated": traversal.root_cause_unstated,
        },
    }))
}
