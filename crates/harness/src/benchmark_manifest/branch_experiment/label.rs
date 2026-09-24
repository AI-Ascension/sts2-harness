// SPDX-License-Identifier: MIT

//! Bounded label shapes for a branch-experiment declaration.
//!
//! A label is the only free-form identity a declaration carries: it names the experiment, the
//! children of the fork and the shared fork-point axes. Every label is bounded, non-empty and
//! NUL-free, and a child label carries a tighter bound than the others because its derived trial
//! key and context namespace add a revision prefix, a separator and a namespace prefix on top of
//! it. Keeping that arithmetic in one place means a declaration that validates always derives a
//! key and a namespace that the rest of the module accepts.

/// Maximum bytes of a branch-experiment, child, trial or context label.
pub const MAX_BRANCH_LABEL_BYTES: usize = 256;
/// Prefix of the fresh per-trial provider/context namespace.
pub const CONTEXT_NAMESPACE_PREFIX: &str = "branch-trial:";
/// Separator between the parts of a stable trial key.
pub const TRIAL_KEY_SEPARATOR: char = '/';
/// Bytes a derived trial key adds to a child label: a 64-hex revision and a separator.
const TRIAL_KEY_PREFIX_BYTES: usize = 64 + 1;
/// Maximum bytes of a child label so its derived trial key and context namespace stay within
/// [`MAX_BRANCH_LABEL_BYTES`].
pub const MAX_CHILD_LABEL_BYTES: usize =
    MAX_BRANCH_LABEL_BYTES - CONTEXT_NAMESPACE_PREFIX.len() - TRIAL_KEY_PREFIX_BYTES;

/// Reports whether a bounded label is non-empty, within its bound and NUL-free.
#[must_use]
pub(crate) fn label_ok(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_BRANCH_LABEL_BYTES && !value.contains('\0')
}

/// Reports whether a child label stays within the bound that keeps its derived trial key and
/// context namespace inside [`MAX_BRANCH_LABEL_BYTES`].
///
/// A child label is not only a label: [`trial_key`](super::plan::trial_key) prefixes it with the
/// 64-hex experiment revision and a separator, and a trial's context namespace prefixes the key
/// with [`CONTEXT_NAMESPACE_PREFIX`]. Bounding the label here means a declaration that validates
/// always derives a key and a namespace the rest of the module accepts, instead of validating and
/// then being refused by every outcome check.
#[must_use]
pub(crate) fn child_label_ok(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_CHILD_LABEL_BYTES && !value.contains('\0')
}

/// Reports whether a settings digest is a lowercase-hex SHA-256 with its algorithm prefix.
#[must_use]
pub(crate) fn digest_ok(value: &str) -> bool {
    let hex = value.strip_prefix("sha256:").unwrap_or("");
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
