// SPDX-License-Identifier: MIT

//! Source doubles for the semantic history read-port suites.
//!
//! Every double answers from synthetic in-memory history and reaches no host: one drifts the owner
//! scope it reports, one answers each request with a summary, one with a page, and one records the
//! requests it was asked while answering none of them.

#![allow(dead_code)]

use std::cell::Cell;

use sts2_harness::semantic_history::{
    SemanticHistoryAgentPort, SemanticHistoryAuthority, SemanticHistoryBinding,
    SemanticHistoryError, SemanticHistoryQuery, SemanticHistoryReader, SemanticHistorySourcePort,
    SemanticHistorySourceRequest, SemanticHistorySourceResponse, SemanticHistoryStore,
    SemanticHistorySummary,
};

use crate::fixture;

/// A source whose reported owner scope is chosen at read time.
pub struct DriftingSource {
    pub store: SemanticHistoryStore,
    pub bindings: [SemanticHistoryBinding; 2],
    pub current: Cell<usize>,
}

impl SemanticHistorySourcePort for DriftingSource {
    fn binding(&self) -> &SemanticHistoryBinding {
        &self.bindings[self.current.get()]
    }

    fn read(
        &self,
        request: &SemanticHistorySourceRequest,
    ) -> Result<SemanticHistorySourceResponse, SemanticHistoryError> {
        self.store.read(request)
    }
}

/// A source that answers every request with a summary, whatever was asked.
pub struct SummaryOnlySource {
    pub binding: SemanticHistoryBinding,
}

impl SemanticHistorySourcePort for SummaryOnlySource {
    fn binding(&self) -> &SemanticHistoryBinding {
        &self.binding
    }

    fn read(
        &self,
        request: &SemanticHistorySourceRequest,
    ) -> Result<SemanticHistorySourceResponse, SemanticHistoryError> {
        let branch_id = match request {
            SemanticHistorySourceRequest::Page { query, .. } => query.branch_id.clone(),
            SemanticHistorySourceRequest::Summary { branch_id }
            | SemanticHistorySourceRequest::Explanation { branch_id, .. } => branch_id.clone(),
        };
        Ok(SemanticHistorySourceResponse::Summary(Box::new(
            SemanticHistorySummary {
                branch_id,
                total: 0,
                captured: 0,
                gaps: 0,
                first_sequence: None,
                last_sequence: None,
            },
        )))
    }
}

/// A source that answers every request with a page, whatever was asked.
pub struct PageOnlySource {
    pub store: SemanticHistoryStore,
    pub binding: SemanticHistoryBinding,
}

/// A source that records the requests it was asked, and answers none of them.
pub struct RecordingSource {
    pub binding: SemanticHistoryBinding,
    pub asked: std::cell::RefCell<Vec<SemanticHistorySourceRequest>>,
}

impl SemanticHistorySourcePort for RecordingSource {
    fn binding(&self) -> &SemanticHistoryBinding {
        &self.binding
    }

    fn read(
        &self,
        request: &SemanticHistorySourceRequest,
    ) -> Result<SemanticHistorySourceResponse, SemanticHistoryError> {
        self.asked.borrow_mut().push(request.clone());
        Err(SemanticHistoryError::Port)
    }
}

impl SemanticHistorySourcePort for PageOnlySource {
    fn binding(&self) -> &SemanticHistoryBinding {
        &self.binding
    }

    fn read(
        &self,
        request: &SemanticHistorySourceRequest,
    ) -> Result<SemanticHistorySourceResponse, SemanticHistoryError> {
        let branch_id = match request {
            SemanticHistorySourceRequest::Page { query, .. } => query.branch_id.clone(),
            SemanticHistorySourceRequest::Summary { branch_id }
            | SemanticHistorySourceRequest::Explanation { branch_id, .. } => branch_id.clone(),
        };
        let reader = SemanticHistoryReader::open(&self.store, &branch_id)?;
        let page = reader.page(&SemanticHistoryQuery::branch(&branch_id, 1), None)?;
        Ok(SemanticHistorySourceResponse::Page(Box::new(page)))
    }
}

pub fn granted(store: &SemanticHistoryStore) -> SemanticHistoryAgentPort<'_, SemanticHistoryStore> {
    SemanticHistoryAgentPort::grant(
        store,
        SemanticHistoryAuthority::HarnessOwned,
        fixture::binding(),
    )
    .expect("a harness-owned grant")
}
