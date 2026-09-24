// SPDX-License-Identifier: MIT

//! Bounded, paged research reads and the coverage each page reports.
//!
//! A read is admitted only through a grant that binds the exact target and the
//! consumer lane, and only for the groups that grant admitted. The page bound is
//! declared, so one research read cannot pull an unbounded amount of hidden
//! state; a page beyond the bound is `InvalidPage` rather than a truncation that
//! looks complete.
//!
//! Admission and reporting are deliberately separate. The harness owns no native
//! capture reader, so it must never *invent* a field's availability: admission
//! returns an [`AdmittedResearchRead`] naming exactly which fields the owner is
//! allowed to report, and [`ResearchPageReport::verify`] checks the owner's
//! report back against that admission. A report that adds, omits, reorders or
//! mislabels a field is refused, so a value can only ever come from the owner
//! that holds the capture, and a consumer never receives a partial result
//! labelled complete.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    FieldAvailability, ResearchFieldRef, ResearchInspectionError, ResearchInspectionGrant,
    ResearchTargetBinding,
};

/// Maximum fields one research page may report.
pub const MAX_RESEARCH_PAGE_ITEMS: usize = 128;

/// One bounded read of selected private fields from a bound checkpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchReadRequest {
    /// The research schema version this request was authored against.
    pub schema_version: String,
    /// The exact checkpoint, run and branch to read.
    pub target: ResearchTargetBinding,
    /// The research identity the operator admitted.
    pub research_id: String,
    /// The lane making the read; must be the grant's admitted consumer.
    pub consumer_lane: String,
    /// The fields to read, in the order the caller wants them reported.
    pub fields: Vec<ResearchFieldRef>,
    /// The zero-based page index, bounded by `max_items`.
    pub page_index: u32,
    /// The requested page size, at most [`MAX_RESEARCH_PAGE_ITEMS`].
    pub max_items: u32,
}

impl ResearchReadRequest {
    /// Validates the request's shape and bounds before any scope check.
    ///
    /// Refuses an unsupported schema version, an unusable field reference, an
    /// empty or oversized selection, or page bounds outside the declared
    /// maximum. Nothing here reads a checkpoint.
    pub fn validate(&self) -> Result<(), ResearchInspectionError> {
        if self.schema_version != super::RESEARCH_INSPECTION_SCHEMA_VERSION {
            return Err(ResearchInspectionError::Incompatible);
        }
        self.target.validate()?;
        if !super::is_research_identity(&self.research_id)
            || !super::is_research_identity(&self.consumer_lane)
        {
            return Err(ResearchInspectionError::InvalidScope);
        }
        if self.fields.is_empty() || self.fields.len() > MAX_RESEARCH_PAGE_ITEMS {
            return Err(ResearchInspectionError::TooManyFields);
        }
        for field in &self.fields {
            ResearchFieldRef::new(field.group, field.field.clone())?;
        }
        if self.max_items == 0 || self.max_items as usize > MAX_RESEARCH_PAGE_ITEMS {
            return Err(ResearchInspectionError::InvalidPage);
        }
        Ok(())
    }

    /// The distinct groups this request names.
    #[must_use]
    pub fn groups(&self) -> BTreeSet<super::ResearchFieldGroup> {
        self.fields.iter().map(ResearchFieldRef::group).collect()
    }
}

/// One reported field and its availability.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchFieldReport {
    /// The field this report describes.
    pub field: ResearchFieldRef,
    /// What the capture could report for it.
    pub availability: FieldAvailability,
}

/// The owner's answer for one page, checked against the admitted slice.
///
/// Every field the admission named must appear exactly once, in the admitted
/// order, with an availability the contract's vocabulary accepts. The owner may
/// report `NotMaterialized`, `SimulationRequired` or `Unsupported` for a field
/// it cannot answer — that is an honest coverage answer, and it is never
/// rewritten into a value here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchPageReport {
    /// The zero-based page index this report answers.
    pub page_index: u32,
    /// One entry per admitted field, in the admitted order.
    pub entries: Vec<ResearchFieldReport>,
}

impl ResearchPageReport {
    /// Checks this owner report against one admitted page.
    ///
    /// Refuses a report for another page, a report whose field set differs from
    /// the admission, and a report whose entries are out of the admitted order.
    /// The check is structural: it proves the owner answered exactly the
    /// admitted slice and no more.
    pub fn verify(&self, admitted: &AdmittedResearchRead) -> Result<(), ResearchInspectionError> {
        if self.page_index != admitted.page_index() {
            return Err(ResearchInspectionError::InvalidPage);
        }
        if self.entries.len() != admitted.len() {
            return Err(ResearchInspectionError::ProtectedField);
        }
        let expected = admitted.fields();
        let answered: Vec<&ResearchFieldRef> = self.entries.iter().map(|e| &e.field).collect();
        if answered.len() != expected.len()
            || answered
                .iter()
                .zip(expected.iter())
                .any(|(left, right)| *left != right)
        {
            return Err(ResearchInspectionError::ProtectedField);
        }
        Ok(())
    }
}

/// One admitted, bounded page of private fields an owner may report.
///
/// Carries the exact field slice, its page position and whether it is the whole
/// answer, so a consumer can never read a partial page as complete. The
/// admission reads no checkpoint: it says what may be asked, not what the answer
/// is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedResearchRead {
    fields: Vec<ResearchFieldRef>,
    page_index: u32,
    total_pages: u32,
    complete: bool,
}

impl AdmittedResearchRead {
    /// The admitted fields, in the order the owner must answer them.
    #[must_use]
    pub fn fields(&self) -> &[ResearchFieldRef] {
        &self.fields
    }

    /// How many fields this page admits.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether this page admits no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The zero-based index of this page.
    #[must_use]
    pub const fn page_index(&self) -> u32 {
        self.page_index
    }

    /// How many pages the admitted request produces.
    #[must_use]
    pub const fn total_pages(&self) -> u32 {
        self.total_pages
    }

    /// Whether this page is the entire admitted answer.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }
}

/// Admits one research read against a grant and returns its bounded page.
///
/// The check order is fixed: request shape and bounds, then the grant's research
/// identity, exact target and consumer lane, then the group scope, then the page.
/// A field outside the admitted groups is `ProtectedField`, so asking for a wider
/// visibility parameter is a refusal rather than a scope change.
///
/// The call is a contract only: it reads no checkpoint, restores nothing and
/// mutates nothing. The owner that holds the capture answers through
/// [`ResearchPageReport`], which must match this admission.
pub fn admit_research_read(
    grant: &ResearchInspectionGrant,
    request: &ResearchReadRequest,
) -> Result<AdmittedResearchRead, ResearchInspectionError> {
    request.validate()?;
    if request.research_id != grant.research_id() {
        return Err(ResearchInspectionError::WrongTarget);
    }
    grant.authorizes(&request.target, &request.consumer_lane)?;
    for field in &request.fields {
        if !grant.admits(field.group) {
            return Err(ResearchInspectionError::ProtectedField);
        }
    }
    let max_items = request.max_items as usize;
    let total_pages: u32 = request
        .fields
        .len()
        .div_ceil(max_items)
        .try_into()
        .map_err(|_| ResearchInspectionError::InvalidPage)?;
    if request.page_index >= total_pages {
        return Err(ResearchInspectionError::InvalidPage);
    }
    let start = request.page_index as usize * max_items;
    let end = (start + max_items).min(request.fields.len());
    let fields = request.fields[start..end].to_vec();
    Ok(AdmittedResearchRead {
        complete: start == 0 && end == request.fields.len(),
        fields,
        page_index: request.page_index,
        total_pages,
    })
}
