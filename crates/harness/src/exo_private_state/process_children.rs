// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;

use rustix::process::getpid;

const MAX_TASKS: usize = 256;
const MAX_CHILDREN: usize = 256;
const MAX_PID_DECIMAL_BYTES: usize = 10;
const MAX_CHILDREN_FILE_BYTES: usize = MAX_CHILDREN * (MAX_PID_DECIMAL_BYTES + 1);

pub(super) fn children_across_tasks() -> Result<BTreeSet<u32>, &'static str> {
    let bridge_pid = getpid().as_raw_nonzero().get() as u32;
    let tasks = fs::read_dir(format!("/proc/{bridge_pid}/task"))
        .map_err(|_| "exo_private_process_tasks")?;
    let mut children = BTreeSet::new();
    let mut task_count = 0_usize;
    for task in tasks {
        task_count = task_count
            .checked_add(1)
            .filter(|count| *count <= MAX_TASKS)
            .ok_or("exo_private_process_bound")?;
        let task = task.map_err(|_| "exo_private_process_tasks")?;
        let tid = task
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or("exo_private_process_tasks")?;
        let file = File::open(format!("/proc/{bridge_pid}/task/{tid}/children"))
            .map_err(|_| "exo_private_process_tasks")?;
        for pid in read_child_ids(file, bridge_pid)? {
            children.insert(pid);
            if children.len() > MAX_CHILDREN {
                return Err("exo_private_process_bound");
            }
        }
    }
    Ok(children)
}

fn read_child_ids(reader: impl Read, bridge_pid: u32) -> Result<Vec<u32>, &'static str> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_CHILDREN_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "exo_private_process_tasks")?;
    if bytes.len() > MAX_CHILDREN_FILE_BYTES {
        return Err("exo_private_process_bound");
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "exo_private_process_children")?;
    let mut children = Vec::with_capacity(MAX_CHILDREN);
    for value in text.split_whitespace() {
        if children.len() == MAX_CHILDREN {
            return Err("exo_private_process_bound");
        }
        let pid = value
            .parse::<u32>()
            .ok()
            .filter(|pid| *pid != 0 && *pid != bridge_pid)
            .ok_or("exo_private_process_children")?;
        children.push(pid);
    }
    Ok(children)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use std::io;

    struct CountingReader {
        bytes: Vec<u8>,
        position: usize,
    }

    impl Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer
                .len()
                .min(self.bytes.len().saturating_sub(self.position));
            buffer[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
            self.position += count;
            Ok(count)
        }
    }

    #[test]
    fn bounded_child_reader_accepts_256_max_width_pids_at_byte_limit() {
        let mut input = String::new();
        for pid in 1..=MAX_CHILDREN {
            input.push_str(&format!("{pid:010} "));
        }
        assert_eq!(input.len(), MAX_CHILDREN_FILE_BYTES);
        let children =
            read_child_ids(input.as_bytes(), u32::MAX).expect("256 bounded PIDs are accepted");
        assert_eq!(children.len(), MAX_CHILDREN);
        assert_eq!(children.first(), Some(&1));
        assert_eq!(children.last(), Some(&(MAX_CHILDREN as u32)));
    }

    #[test]
    fn bounded_child_reader_rejects_invalid_pid() {
        assert_eq!(
            read_child_ids(&b"4294967296"[..], u32::MAX).err(),
            Some("exo_private_process_children")
        );
    }

    #[test]
    fn bounded_child_reader_rejects_a_257th_pid() {
        let input = (1..=MAX_CHILDREN + 1)
            .map(|pid| pid.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            read_child_ids(input.as_bytes(), u32::MAX).err(),
            Some("exo_private_process_bound")
        );
    }

    #[test]
    fn oversized_child_list_reads_only_the_limit_plus_one_sentinel() {
        let mut reader = CountingReader {
            bytes: vec![b'1'; MAX_CHILDREN_FILE_BYTES * 1024],
            position: 0,
        };
        assert_eq!(
            read_child_ids(&mut reader, u32::MAX).err(),
            Some("exo_private_process_bound")
        );
        assert_eq!(reader.position, MAX_CHILDREN_FILE_BYTES + 1);
    }
}
