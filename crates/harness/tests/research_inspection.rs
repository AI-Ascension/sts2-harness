// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Focused admission/isolation matrix for scoped research inspection of hidden
//! checkpoint state (issue #129, T1/T2). Every case is synthetic: it exercises
//! the source-only contract and asserts no native game effect. The native
//! agreement and capture-manifest checks stay with the native owner and remain
//! an open gate.

use sts2_harness::research_inspection::{
    AdmittedResearchRead, FieldAvailability, MAX_FIELD_REF_BYTES, MAX_RESEARCH_GRANTS,
    MAX_RESEARCH_IDENTITY_BYTES, MAX_RESEARCH_PAGE_ITEMS, RESEARCH_INSPECTION_SCHEMA_VERSION,
    ResearchFieldGroup, ResearchFieldRef, ResearchFieldReport, ResearchInspectionError,
    ResearchInspectionGrant, ResearchPageReport, ResearchReadRequest, ResearchScopeRefusal,
    ResearchTargetBinding, admit_research_read, is_research_identity,
};

const RESEARCH_ID: &str = "research-1";
const CONSUMER: &str = "research-lane-1";
const CHECKPOINT: &str = "checkpoint-1";
const RUN: &str = "run-1";
const BRANCH: &str = "branch-1";

fn binding(checkpoint: &str, run: &str, branch: &str) -> ResearchTargetBinding {
    ResearchTargetBinding {
        checkpoint_id: checkpoint.to_owned(),
        run_id: run.to_owned(),
        branch_id: branch.to_owned(),
    }
}

fn target() -> ResearchTargetBinding {
    binding(CHECKPOINT, RUN, BRANCH)
}

fn grant(groups: impl IntoIterator<Item = ResearchFieldGroup>) -> ResearchInspectionGrant {
    ResearchInspectionGrant::admit(RESEARCH_ID, target(), CONSUMER, groups).expect("grant admits")
}

fn default_grant() -> ResearchInspectionGrant {
    grant(ResearchFieldGroup::supported())
}

fn field(group: ResearchFieldGroup, name: &str) -> ResearchFieldRef {
    ResearchFieldRef::new(group, name).expect("field reference is usable")
}

fn request(fields: Vec<ResearchFieldRef>, page_index: u32, max_items: u32) -> ResearchReadRequest {
    ResearchReadRequest {
        schema_version: RESEARCH_INSPECTION_SCHEMA_VERSION.to_owned(),
        target: target(),
        research_id: RESEARCH_ID.to_owned(),
        consumer_lane: CONSUMER.to_owned(),
        fields,
        page_index,
        max_items,
    }
}

fn report(
    admitted: &AdmittedResearchRead,
    entries: Vec<ResearchFieldReport>,
) -> ResearchPageReport {
    ResearchPageReport {
        page_index: admitted.page_index(),
        entries,
    }
}

fn available(group: ResearchFieldGroup, name: &str, value: &str) -> ResearchFieldReport {
    ResearchFieldReport {
        field: field(group, name),
        availability: FieldAvailability::Available {
            value: value.to_owned(),
        },
    }
}

#[path = "research_inspection/admission.rs"]
mod admission;

#[path = "research_inspection/isolation.rs"]
mod isolation;

#[path = "research_inspection/coverage.rs"]
mod coverage;

#[path = "research_inspection/bounds.rs"]
mod bounds;

#[path = "research_inspection/matrix.rs"]
mod matrix;
