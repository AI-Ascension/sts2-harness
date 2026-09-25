// SPDX-License-Identifier: MIT

//! Reads the byte bounds the shipped Exo entrypoints enforce, so the bound oracle cannot pin a
//! value the tree no longer ships.
//!
//! `bound_oracle` declares both bounds as constants, and nothing else in the tree compares those
//! constants with the sources they mirror: the record gate binds only the oracle's own paths, and
//! the workflow's `jq` asserts compare the report against the test's literals. A committed move of
//! a shipped bound would therefore leave a green report pinning a bound that no longer exists —
//! the same shape as an inert, misspelled refusal. These readers close that gap by parsing the
//! declarations the entrypoints actually enforce, and each fails closed if the source is restated
//! in a form the reader does not understand rather than silently returning nothing.

use super::Result;
use std::path::Path;

/// Evaluates a `N * M` byte-count expression taken from a shipped declaration.
fn evaluate_bytes(expression: &str, label: &str) -> Result<usize> {
    let mut product: usize = 1;
    for term in expression.split('*') {
        let digits = term.trim().replace('_', "");
        let value: usize = digits
            .parse()
            .map_err(|_| format!("{label}: {term:?} is not a decimal byte count"))?;
        product = product
            .checked_mul(value)
            .ok_or_else(|| format!("{label}: {expression:?} overflows a usize"))?;
    }
    Ok(product)
}

/// The isolated executor's read bound, read from its shipped declaration.
pub fn shipped_input_limit(root: &Path) -> Result<usize> {
    let source = std::fs::read_to_string(root.join("experiments/exo-agent/bridge/src/main.rs"))?;
    let expression = source
        .lines()
        .find_map(|line| line.trim().strip_prefix("const INPUT_LIMIT: u64 = "))
        .and_then(|rest| rest.strip_suffix(';'))
        .ok_or("INPUT_LIMIT is not declared in the isolated executor")?;
    evaluate_bytes(expression, "executor INPUT_LIMIT")
}

/// The bridge's request-parse bound, read from its shipped call site.
///
/// See [`shipped_input_limit`] for why the oracle compares rather than assumes this value.
pub fn shipped_bridge_request_bound(root: &Path) -> Result<usize> {
    let source = std::fs::read_to_string(root.join("crates/harness/src/bin/sts2-exo-bridge.rs"))?;
    let argument = source
        .lines()
        .find_map(|line| line.split_once("parse_bridge_request_envelope(&bytes, "))
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(argument, _)| argument.trim())
        .ok_or("the bridge does not parse its request envelope with a byte bound")?;
    evaluate_bytes(argument, "bridge request bound")
}

/// Fails unless both shipped bounds still equal the values the oracle pins.
///
/// Call this before anything is driven: a moved bound must fail the oracle rather than leave a
/// green report that pins a bound the tree no longer enforces.
pub fn pinned_bounds(
    root: &Path,
    pinned_input_limit: usize,
    pinned_request_bound: usize,
) -> Result<ShippedBounds> {
    let input_limit = shipped_input_limit(root)?;
    let request_bound = shipped_bridge_request_bound(root)?;
    if input_limit != pinned_input_limit || request_bound != pinned_request_bound {
        return Err(format!(
            "the shipped bounds moved: the executor reads {input_limit} and the bridge parses \
             {request_bound}, while the oracle pins {pinned_input_limit} and {pinned_request_bound}"
        )
        .into());
    }
    Ok(ShippedBounds {
        input_limit,
        request_bound,
    })
}

/// The shipped bounds, once verified against the values the oracle pins.
pub struct ShippedBounds {
    pub input_limit: usize,
    pub request_bound: usize,
}
