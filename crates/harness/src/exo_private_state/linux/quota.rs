// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn scan_policy(
    roots: &[PolicyRoot],
    policy_locks: &[PolicyLock],
    maximum_bytes: u64,
) -> Result<u64, &'static str> {
    verify_policy_locks(roots, policy_locks)?;
    let mut entries = 0_usize;
    let mut total = 0_u64;
    for (root, lock) in roots.iter().zip(policy_locks) {
        entries = entries.checked_add(1).ok_or("exo_private_entry_bound")?;
        if entries > super::super::fs::MAX_TREE_ENTRIES {
            return Err("exo_private_entry_bound");
        }
        let lock_size = lock
            .file
            .metadata()
            .map_err(|_| "exo_private_policy_lock")?
            .len();
        if lock_size > MAX_POLICY_LOCK_BYTES {
            return Err("exo_private_policy_owner");
        }
        total = total.checked_add(lock_size).ok_or("exo_private_quota")?;
        if total > maximum_bytes {
            return Err("exo_private_quota");
        }
        scan_policy_root(root, true, &mut entries, &mut total, maximum_bytes)?;
    }
    Ok(total)
}

pub(super) fn verify_policy_locks(
    roots: &[PolicyRoot],
    policy_locks: &[PolicyLock],
) -> Result<(), &'static str> {
    if roots.len() != policy_locks.len() {
        return Err("exo_private_policy_lock");
    }
    for (root, lock) in roots.iter().zip(policy_locks) {
        verify_policy_lock(root, lock)?;
    }
    Ok(())
}

pub(super) fn reject_aliased_roots(roots: &[PolicyRoot]) -> Result<(), &'static str> {
    for left in 0..roots.len() {
        for right in (left + 1)..roots.len() {
            if roots[left].identity.device == roots[right].identity.device
                && roots[left].identity.inode == roots[right].identity.inode
                || roots[left].ancestors.iter().any(|identity| {
                    identity.device == roots[right].identity.device
                        && identity.inode == roots[right].identity.inode
                })
                || roots[right].ancestors.iter().any(|identity| {
                    identity.device == roots[left].identity.device
                        && identity.inode == roots[left].identity.inode
                })
            {
                return Err("exo_private_root_alias");
            }
        }
    }
    Ok(())
}
