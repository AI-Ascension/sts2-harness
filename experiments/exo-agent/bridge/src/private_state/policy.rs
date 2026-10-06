// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn policy_digest(policy: &Policy) -> String {
    let mut value = String::from("sts2.exo-private-state-v1\u{1f}");
    push_field(&mut value, &policy.state_root);
    push_field(&mut value, &policy.cache_root);
    push_field(&mut value, &policy.temp_root);
    push_field(&mut value, &policy.quota_bytes.to_string());
    push_field(&mut value, &policy.max_retention_days.to_string());
    push_field(&mut value, &format!("{:o}", policy.permissions_octal));
    let digest = Sha256::digest(value.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn push_field(output: &mut String, value: &str) {
    output.push_str(&value.len().to_string());
    output.push(':');
    output.push_str(value);
    output.push('\u{1f}');
}

pub(super) fn read_boot_id() -> Result<String, &'static str> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| "exo_private_process_identity")?;
    let boot_id = boot_id.trim();
    if boot_id.len() != 36
        || !boot_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("exo_private_process_identity");
    }
    Ok(boot_id.to_ascii_lowercase())
}

pub(super) fn proc_identity(pid: u32) -> Result<ProcIdentity, &'static str> {
    let value = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| "exo_private_process_identity")?;
    let suffix = value
        .rfind(')')
        .and_then(|index| value.get(index + 1..))
        .ok_or("exo_private_process_identity")?;
    let fields = suffix.split_whitespace().collect::<Vec<_>>();
    Ok(ProcIdentity {
        parent_pid: parse_proc_field(&fields, 1)?,
        process_group: parse_proc_field(&fields, 2)?,
        session: parse_proc_field(&fields, 3)?,
        start_time_ticks: parse_proc_field(&fields, 19)?,
    })
}

fn parse_proc_field<T: std::str::FromStr>(
    fields: &[&str],
    index: usize,
) -> Result<T, &'static str> {
    fields
        .get(index)
        .and_then(|value| value.parse().ok())
        .ok_or("exo_private_process_identity")
}

pub(super) fn valid_attempt_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
