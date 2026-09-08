// SPDX-License-Identifier: MIT

mod schema;

use schema::{allows_null, child_kind, is_allowed, validate_shape};
use std::collections::BTreeSet;

use serde_json::{Map, Value};

const MAX_OBSERVATION_BYTES: usize = 128 * 1024;
const MAX_TEXT_BYTES: usize = 512;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_CARDS: usize = 256;
const MAX_ENEMIES: usize = 64;
const MAX_LEGAL_ACTIONS: usize = 256;
const MAX_SHOP_ITEMS: usize = 128;
const MAX_TEXT_ITEMS: usize = 256;

include!("sandbox_api.rs");
include!("sandbox_validator.rs");
