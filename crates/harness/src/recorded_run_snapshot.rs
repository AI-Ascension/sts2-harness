// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use rustix::fs::{Mode, OFlags, open, openat};
use std::os::unix::fs::MetadataExt;

use super::MAX_FILE_BYTES;
use crate::sha256_hex;

type Stamp = (u64, u64, i64, i64, i64, i64, String);

pub(super) struct Snapshot {
    root: File,
    pub files: BTreeMap<String, Vec<u8>>,
    stamps: BTreeMap<String, Stamp>,
}
impl Snapshot {
    pub fn read(path: &Path) -> Result<Self, String> {
        let root = File::from(
            open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| String::from("invalid_source_root"))?,
        );
        let mut snapshot = Self {
            root,
            files: BTreeMap::new(),
            stamps: BTreeMap::new(),
        };
        let mut remaining = 64 * 1024 * 1024;
        for name in [
            "manifest.json",
            "result.json",
            "trajectory.jsonl",
            "decisions.jsonl",
            "mcp.jsonl",
            "provider-accounting.jsonl",
        ] {
            if name == "provider-accounting.jsonl"
                && path
                    .join(name)
                    .symlink_metadata()
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            let (bytes, stamp) = snapshot.read_child(name, remaining)?;
            remaining -= bytes.len();
            snapshot.files.insert(name.to_owned(), bytes);
            snapshot.stamps.insert(name.to_owned(), stamp);
        }
        Ok(snapshot)
    }
    fn read_child(&self, name: &str, remaining: usize) -> Result<(Vec<u8>, Stamp), String> {
        let mut file = File::from(
            openat(
                &self.root,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|_| String::from("source_child_not_regular"))?,
        );
        let before = file
            .metadata()
            .map_err(|_| String::from("source_metadata"))?;
        if !before.is_file() || before.len() > MAX_FILE_BYTES.min(remaining) as u64 {
            return Err(String::from("source_file_limit_or_type"));
        }
        let mut bytes = vec![0; before.len() as usize];
        file.read_exact(&mut bytes)
            .map_err(|_| String::from("source_read"))?;
        if file
            .read(&mut [0u8; 1])
            .map_err(|_| String::from("source_read"))?
            != 0
        {
            return Err(String::from("source_changed"));
        }
        let after = file
            .metadata()
            .map_err(|_| String::from("source_metadata"))?;
        if bytes.len() > MAX_FILE_BYTES
            || before.len() != after.len()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.mtime() != after.mtime()
            || before.ctime() != after.ctime()
        {
            return Err(String::from("source_changed"));
        }
        let stamp = (
            after.dev(),
            after.ino(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
            sha256_hex(&bytes),
        );
        Ok((bytes, stamp))
    }
    pub fn verify(&self, path: &Path) -> Result<(), String> {
        let before = self
            .root
            .metadata()
            .map_err(|_| String::from("source_root"))?;
        let after = path
            .symlink_metadata()
            .map_err(|_| String::from("source_root"))?;
        if !after.is_dir() || before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(String::from("source_root_changed"));
        }
        if !self.files.contains_key("provider-accounting.jsonl")
            && !path
                .join("provider-accounting.jsonl")
                .symlink_metadata()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(String::from("source_inventory_changed"));
        }
        for (name, stamp) in &self.stamps {
            if self.read_child(name, MAX_FILE_BYTES)?.1 != *stamp {
                return Err(String::from("source_changed"));
            }
        }
        Ok(())
    }
    pub fn required(&self, name: &str) -> Result<&[u8], String> {
        self.files
            .get(name)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("missing_source: {name}"))
    }
}
