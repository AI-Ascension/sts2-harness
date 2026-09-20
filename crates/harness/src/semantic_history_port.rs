// SPDX-License-Identifier: MIT

//! The single supported read path, and the refusal that closes the bypass.

use super::{
    Error, SemanticHistoryAuthority, SemanticHistoryBinding, SemanticHistoryContinuation,
    SemanticHistoryExplanation, SemanticHistoryPage, SemanticHistoryQuery, SemanticHistoryReader,
    SemanticHistoryStore, SemanticHistorySummary, SemanticHistoryTraversalLimits,
    validate_history_identity,
};

/// What a caller may ask the history source for.
///
/// The request vocabulary is closed and contains no storage coordinates. There is deliberately no
/// way to name a path, a bucket or an artifact: a request that could address storage directly would
/// let a caller reach history the owner did not grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticHistorySourceRequest {
    /// Read one bounded page.
    Page {
        /// The bounded question.
        query: SemanticHistoryQuery,
        /// Where to resume, when the caller is following a multi-page read.
        continuation: Option<SemanticHistoryContinuation>,
    },
    /// Summarize one branch.
    Summary {
        /// The branch to summarize.
        branch_id: String,
    },
    /// Explain why one observed change happened.
    Explanation {
        /// The branch the event was recorded on.
        branch_id: String,
        /// The event the question starts from.
        event_id: String,
        /// The bounds the walk must stay inside.
        limits: SemanticHistoryTraversalLimits,
    },
}

/// What a source answers with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticHistorySourceResponse {
    /// A page of history.
    Page(Box<SemanticHistoryPage>),
    /// A branch summary.
    Summary(Box<SemanticHistorySummary>),
    /// A why-changed explanation.
    Explanation(Box<SemanticHistoryExplanation>),
}

/// The owned source of history, implemented by the harness store.
///
/// This is the port an agent reads through. Implementing it elsewhere does not grant history
/// authority: the refusal that matters is on the caller side, in `SemanticHistoryAgentPort`.
pub trait SemanticHistorySourcePort {
    /// The owner scope this source serves.
    fn binding(&self) -> &SemanticHistoryBinding;

    /// Answers one bounded request.
    fn read(
        &self,
        request: &SemanticHistorySourceRequest,
    ) -> Result<SemanticHistorySourceResponse, Error>;
}

impl SemanticHistorySourcePort for SemanticHistoryStore {
    fn binding(&self) -> &SemanticHistoryBinding {
        SemanticHistoryStore::binding(self)
    }

    fn read(
        &self,
        request: &SemanticHistorySourceRequest,
    ) -> Result<SemanticHistorySourceResponse, Error> {
        match request {
            SemanticHistorySourceRequest::Page {
                query,
                continuation,
            } => {
                let reader = SemanticHistoryReader::open(self, &query.branch_id)?;
                let page = reader.page(query, continuation.as_ref())?;
                Ok(SemanticHistorySourceResponse::Page(Box::new(page)))
            }
            SemanticHistorySourceRequest::Summary { branch_id } => {
                let reader = SemanticHistoryReader::open(self, branch_id)?;
                let summary = reader.summary(branch_id)?;
                Ok(SemanticHistorySourceResponse::Summary(Box::new(summary)))
            }
            SemanticHistorySourceRequest::Explanation {
                branch_id,
                event_id,
                limits,
            } => {
                let explanation = self.explain(branch_id, event_id, *limits)?;
                Ok(SemanticHistorySourceResponse::Explanation(Box::new(
                    explanation,
                )))
            }
        }
    }
}

/// The port an agent reads history through.
///
/// The adapter is deliberately not an authority over storage. It may call the owned source port and
/// it may not reach around it, so a game adapter cannot read an arbitrary artifact, and it cannot
/// reverse-call the harness to obtain one either: the only thing it holds is a source port whose
/// vocabulary contains no storage coordinates.
pub struct SemanticHistoryAgentPort<'a, S: SemanticHistorySourcePort> {
    source: &'a S,
    authority: SemanticHistoryAuthority,
    expected: SemanticHistoryBinding,
}

impl<'a, S: SemanticHistorySourcePort> SemanticHistoryAgentPort<'a, S> {
    /// Grants an agent read access through one owned source.
    ///
    /// The authority must be `HarnessOwned`. A caller that asks for `NotGranted` — that is, a caller
    /// that wants to read storage itself — is refused here rather than given a port it could use to
    /// bypass the owner.
    pub fn grant(
        source: &'a S,
        authority: SemanticHistoryAuthority,
        expected: SemanticHistoryBinding,
    ) -> Result<Self, Error> {
        if authority != SemanticHistoryAuthority::HarnessOwned {
            return Err(Error::Authority);
        }
        expected.validate()?;
        if !source.binding().same_owner(&expected) {
            return Err(Error::Scope);
        }
        if source.binding().authority_epoch != expected.authority_epoch {
            return Err(Error::Epoch);
        }
        Ok(Self {
            source,
            authority,
            expected,
        })
    }

    /// The authority this port was granted under.
    #[must_use]
    pub const fn authority(&self) -> SemanticHistoryAuthority {
        self.authority
    }

    /// Reads one bounded page through the owned source.
    pub fn page(&self, query: &SemanticHistoryQuery) -> Result<SemanticHistoryPage, Error> {
        self.page_from(query, None)
    }

    /// Reads one bounded page, resuming where a previous page left off.
    ///
    /// Without this a caller could not follow a multi-page read through the port at all: it would be
    /// handed a continuation it had no way to spend, and the only honest answer left would be the
    /// first page over and over.
    pub fn page_from(
        &self,
        query: &SemanticHistoryQuery,
        continuation: Option<&SemanticHistoryContinuation>,
    ) -> Result<SemanticHistoryPage, Error> {
        self.assert_still_granted()?;
        query.validate()?;
        validate_history_identity(&query.branch_id, "query.branch_id")?;
        match self.source.read(&SemanticHistorySourceRequest::Page {
            query: query.clone(),
            continuation: continuation.cloned(),
        })? {
            SemanticHistorySourceResponse::Page(page) => Ok(*page),
            _ => Err(Error::Port),
        }
    }

    /// Summarizes one branch through the owned source.
    pub fn summary(&self, branch_id: &str) -> Result<SemanticHistorySummary, Error> {
        self.assert_still_granted()?;
        validate_history_identity(branch_id, "branch_id")?;
        match self.source.read(&SemanticHistorySourceRequest::Summary {
            branch_id: branch_id.to_owned(),
        })? {
            SemanticHistorySourceResponse::Summary(summary) => Ok(*summary),
            _ => Err(Error::Port),
        }
    }

    /// Explains one observed change through the owned source.
    ///
    /// The walk is bounded by the caller's limits and refuses a cycle, so an adapter cannot ask for
    /// an answer this boundary would have to keep computing.
    pub fn explain(
        &self,
        branch_id: &str,
        event_id: &str,
        limits: SemanticHistoryTraversalLimits,
    ) -> Result<SemanticHistoryExplanation, Error> {
        self.assert_still_granted()?;
        validate_history_identity(branch_id, "branch_id")?;
        validate_history_identity(event_id, "event_id")?;
        limits.validate()?;
        match self
            .source
            .read(&SemanticHistorySourceRequest::Explanation {
                branch_id: branch_id.to_owned(),
                event_id: event_id.to_owned(),
                limits,
            })? {
            SemanticHistorySourceResponse::Explanation(explanation) => Ok(*explanation),
            _ => Err(Error::Port),
        }
    }

    /// Re-checks the grant against the source on every read.
    ///
    /// A grant is not a durable capability: the source must still serve the same owner and the same
    /// authority epoch when the read happens. Checking only at grant time would let a source that
    /// later changed scope or advanced its epoch keep answering through a stale grant.
    fn assert_still_granted(&self) -> Result<(), Error> {
        if !self.source.binding().same_owner(&self.expected) {
            return Err(Error::Scope);
        }
        if self.source.binding().authority_epoch != self.expected.authority_epoch {
            return Err(Error::Epoch);
        }
        Ok(())
    }

    /// Refuses a caller that names storage directly.
    ///
    /// This exists so the bypass is a refusal with a name rather than an absence: a caller asking for
    /// arbitrary artifact storage is told no, and cannot obtain a handle from this boundary.
    pub fn refuse_direct_storage(&self, _coordinate: &str) -> Result<(), Error> {
        Err(Error::Authority)
    }
}
