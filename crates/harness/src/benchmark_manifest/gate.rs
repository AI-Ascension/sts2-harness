// SPDX-License-Identifier: MIT

//! Pre-mutation rerun admission gate.
//!
//! Acceptance criterion 2 of the benchmark-manifest feature (sts2-harness#121)
//! requires that the same displayed seed with different unlocks, profile, game/mod
//! build, act order, character or modifiers is refused as a reproducible rerun
//! *before* the game is mutated. [`Manifest::compare`] already enumerates every
//! difference exactly; this module turns that comparison into an admission decision
//! a pre-mutation allocation path cannot bypass.
//!
//! Ordering guarantee: the gate runs [`Manifest::compare`] to completion and only
//! then, on an empty mismatch set, mints a [`RerunAdmission`]. Because
//! `RerunAdmission` has a private constructor, any allocation/launch path that takes
//! `&RerunAdmission` (see [`RerunAllocationSeam`]) cannot run before a successful
//! comparison; [`admit_and_allocate`] is the single ordering point. The token also
//! owns the exact admitted [`Manifest`] (reachable only through
//! [`RerunAdmission::declaration`]), so the seam is given exactly the admitted
//! declaration as its authoritative input; implementors must allocate only for it.
//!
//! Scope note: no in-repo consumer of the benchmark manifest and no governed
//! benchmark-rerun flow exist yet, so this module is the production pre-mutation
//! contract and its consumer seam rather than a call site attached to an existing
//! allocator. Attaching a real consumer — gating the in-repo seeded-launch path
//! (`runtime_support/runtime_v3_seeded.rs`) on this admission — is owned by the
//! runtime/gateway seeded-launch owner, alongside the native-admission recheck in
//! sts2-harness#103 and sts2-game-mod#79. No new allocator is invented here. A
//! consumer attaches by implementing [`RerunAllocationSeam`] and holding the
//! [`RerunAdmission`] it returns; the compare-before-allocation ordering is then
//! structural.
//!
//! An admitted rerun certifies equal *declarations only*: not native compatibility,
//! not seed durability, and not authorization to mutate. Rechecking the settled
//! native receipt at admission remains owned by sts2-harness#103 and
//! sts2-game-mod#79 (requirement 4's "again at native admission").

use std::fmt;

use serde::Serialize;

use super::{Manifest, Mismatch};

/// The sealing module: only the gate module can name this type, so no other module
/// or crate can forge an admission token.
mod sealed {
    /// Unforgeable proof marker; constructed nowhere outside the gate module.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) struct Admitted;
}

/// Proof that two manifests declared identical controlled inputs.
///
/// Minted only by [`Manifest::admit_rerun`] and [`admit_and_allocate`] after an empty
/// [`Manifest::compare`]. Holding one is necessary and sufficient to cross a
/// [`RerunAllocationSeam`], but it certifies equal declarations only: never native
/// compatibility, seed durability, or authorization to mutate.
///
/// The token owns the admitted declaration and offers it to the seam as the
/// authoritative input; implementors must allocate only for it.
#[derive(Clone, Debug)]
pub struct RerunAdmission {
    _seal: sealed::Admitted,
    declaration: Manifest,
}

impl RerunAdmission {
    /// The honest evidence level this admission carries.
    ///
    /// Deliberately weaker than the public projection's `declared_inputs_only`: equal
    /// declarations permit a rerun attempt, they do not prove reproducible gameplay.
    #[must_use]
    pub const fn evidence(&self) -> &'static str {
        "declared_inputs_equal"
    }

    /// The exact declaration whose [`Manifest::compare`] produced this admission.
    ///
    /// This is the only declaration a [`RerunAllocationSeam`] may allocate for; it
    /// is the immutable artifact compared against the incumbent, never a later or
    /// unrelated manifest.
    #[must_use]
    pub fn declaration(&self) -> &Manifest {
        &self.declaration
    }
}

/// Non-empty, machine-readable refusal. It reflects only the exact mismatch
/// categories and never any supplied field values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RerunRefusal {
    mismatches: Vec<Mismatch>,
}

impl RerunRefusal {
    /// The exact mismatch categories that refused the rerun. Never empty.
    #[must_use]
    pub fn mismatches(&self) -> &[Mismatch] {
        &self.mismatches
    }
}

impl fmt::Display for RerunRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "rerun refused: {:?}", self.mismatches)
    }
}

impl std::error::Error for RerunRefusal {}

impl Manifest {
    /// Compares `candidate` against this immutable declaration and admits the rerun
    /// only when every required field is equal.
    ///
    /// Any difference returns a [`RerunRefusal`] carrying the exact mismatch
    /// categories before any allocation happens. Only an empty [`Manifest::compare`]
    /// set mints a [`RerunAdmission`]; no explicitly versioned compatibility rule
    /// relaxes equality here.
    pub fn admit_rerun(&self, candidate: &Self) -> Result<RerunAdmission, RerunRefusal> {
        let mismatches = self.compare(candidate);
        if mismatches.is_empty() {
            Ok(RerunAdmission {
                _seal: sealed::Admitted,
                declaration: candidate.clone(),
            })
        } else {
            Err(RerunRefusal { mismatches })
        }
    }
}

/// The pre-mutation allocation/launch boundary a governed rerun must cross.
///
/// Implementors receive a [`RerunAdmission`] whose [`RerunAdmission::declaration`]
/// is the only declaration that compared equal. Because that token has a private
/// constructor, an implementation cannot be reached before the declarations compare
/// equal, so the compare-before-allocation ordering is structural rather than
/// conventional. Allocation must read its inputs from the admission token.
pub trait RerunAllocationSeam {
    /// Result of the allocation/launch step.
    type Output;
    /// Failure produced by the allocation/launch step itself.
    type Error;

    /// Performs the pre-mutation allocation. Reached only after an admitted comparison.
    fn allocate(&self, admission: &RerunAdmission) -> Result<Self::Output, Self::Error>;
}

/// Outcome of a gated rerun attempt, keeping refusal distinct from a seam failure.
#[derive(Debug)]
pub enum RerunGateError<E> {
    /// The declarations differed; the seam was never reached.
    Refused(RerunRefusal),
    /// The declarations matched; the allocation seam itself failed.
    Seam(E),
}

impl<E: fmt::Display> fmt::Display for RerunGateError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(formatter),
            Self::Seam(error) => write!(formatter, "rerun allocation failed: {error}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for RerunGateError<E> {}

/// Runs the pre-mutation gate and, only on admission, crosses `seam` exactly once.
///
/// Ordering guarantee: [`Manifest::compare`] runs to completion before
/// [`RerunAllocationSeam::allocate`]. A refusal returns the machine-readable
/// [`RerunRefusal`] without invoking the seam at all.
pub fn admit_and_allocate<S: RerunAllocationSeam>(
    incumbent: &Manifest,
    candidate: &Manifest,
    seam: &S,
) -> Result<S::Output, RerunGateError<S::Error>> {
    let admission = incumbent
        .admit_rerun(candidate)
        .map_err(RerunGateError::Refused)?;
    seam.allocate(&admission).map_err(RerunGateError::Seam)
}
