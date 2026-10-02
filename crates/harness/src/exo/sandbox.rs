// SPDX-License-Identifier: MIT

mod schema;

use schema::{allows_null, child_kind, is_allowed, validate_shape};
use std::collections::BTreeSet;

use serde_json::{Map, Value};

const MAX_OBSERVATION_BYTES: usize = 128 * 1024;
/// Bound on an identity string, in UTF-8 bytes.
///
/// Identities match the contract's `#/$defs/identity` pattern, which admits only
/// ASCII alphanumerics and `._:/-`, so a character bound and a byte bound are
/// equivalent here. Bytes are kept because that is the cheaper precondition to
/// state and cannot drift away from the pattern.
const MAX_IDENTITY_BYTES: usize = 512;
/// Bound on a free-text string, in Unicode characters.
///
/// The contract bounds text and offered attributes with `maxLength`, and JSON
/// Schema `maxLength` counts characters. `MAX_OBSERVATION_BYTES` already caps
/// the encoded projection, so the character bound is the only per-field limit
/// needed, and counting bytes here would make the sandbox refuse a conforming
/// message: 257 `é` is 514 bytes but only 257 characters.
const MAX_TEXT_CHARACTERS: usize = 512;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_CARDS: usize = 256;
const MAX_RELICS: usize = 256;
const MAX_POTIONS: usize = 64;
/// The contract's `#/$defs/disclosed_set` declares `maxItems: 256`.
const MAX_CHOICE_CONTENTS: usize = 256;
const MAX_ENEMIES: usize = 64;
const MAX_LEGAL_ACTIONS: usize = 256;
const MAX_SHOP_ITEMS: usize = 128;
const MAX_TEXT_ITEMS: usize = 256;

include!("sandbox_api.rs");
include!("sandbox_validator.rs");

#[cfg(test)]
#[path = "sandbox_bounds_tests.rs"]
mod bounds_tests;
