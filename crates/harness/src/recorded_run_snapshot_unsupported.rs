// SPDX-License-Identifier: MIT

//! Refuse export where descriptor-relative, no-follow snapshots are unavailable.

use std::collections::BTreeMap;
use std::path::Path;

pub(super) struct Snapshot {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Snapshot {
    pub fn read(_path: &Path) -> Result<Self, String> {
        Err(String::from("source_snapshot_platform_unsupported"))
    }

    pub fn verify(&self, _path: &Path) -> Result<(), String> {
        Err(String::from("source_snapshot_platform_unsupported"))
    }

    pub fn required(&self, _name: &str) -> Result<&[u8], String> {
        Err(String::from("source_snapshot_platform_unsupported"))
    }
}
