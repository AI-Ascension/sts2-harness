// SPDX-License-Identifier: MIT

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

include!("runtime_v3_lifecycle_rest_action_support.rs");
include!("runtime_v3_lifecycle_rest_action_support_tail.rs");
include!("runtime_v3_lifecycle_rest_action_fixture_responses.rs");
include!("runtime_v3_lifecycle_rest_action_fixture.rs");
include!("runtime_v3_lifecycle_rest_action_gateway.rs");
include!("runtime_v3_lifecycle_rest_action_body.rs");
