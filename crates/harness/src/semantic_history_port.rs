// SPDX-License-Identifier: MIT

//! The single supported read path, and the refusal that closes the bypass.

use super::{
    Error, SemanticHistoryAuthority, SemanticHistoryBinding, SemanticHistoryPage,
    SemanticHistoryQuery, SemanticHistoryReader, SemanticHistoryStore, SemanticHistorySummary,
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
    },
    /// Summarize one branch.
    Summary {
        /// The branch to summarize.
        branch_id: String,
    },
}

/// What a source answers with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticHistorySourceResponse {
    /// A page of history.
    Page(Box<SemanticHistoryPage>),
    /// A branch summary.
    Summary(Box<SemanticHistorySummary>),
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
            SemanticHistorySourceRequest::Page { query } => {
                let reader = SemanticHistoryReader::open(self, &query.branch_id)?;
                let page = reader.page(query, None)?;
                Ok(SemanticHistorySourceResponse::Page(Box::new(page)))
            }
            SemanticHistorySourceRequest::Summary { branch_id } => {
                let reader = SemanticHistoryReader::open(self, branch_id)?;
                let summary = reader.summary(branch_id)?;
                Ok(SemanticHistorySourceResponse::Summary(Box::new(summary)))
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
        self.assert_still_granted()?;
        query.validate()?;
        validate_history_identity(&query.branch_id, "query.branch_id")?;
        match self.source.read(&SemanticHistorySourceRequest::Page {
            query: query.clone(),
        })? {
            SemanticHistorySourceResponse::Page(page) => Ok(*page),
            SemanticHistorySourceResponse::Summary(_) => Err(Error::Port),
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
            SemanticHistorySourceResponse::Page(_) => Err(Error::Port),
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
