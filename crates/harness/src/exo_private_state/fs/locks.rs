// SPDX-License-Identifier: MIT

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use rustix::fs::{FlockOperation, Mode, OFlags, openat};
use rustix::io::Errno;

use super::{
    FileIdentity, MAX_POLICY_LOCK_BYTES, POLICY_LOCK_RECORD, PRIVATE_FILE_MODE, PolicyLock,
    PolicyRoot, open_policy_root, same_file, verify_private_file,
};

pub(in crate::exo_private_state) fn lock_policy_roots(
    roots: &[PolicyRoot],
    policy_digest: &str,
) -> Result<Vec<PolicyLock>, &'static str> {
    let mut order = (0..roots.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| (roots[*index].identity.device, roots[*index].identity.inode));
    let mut acquired = Vec::with_capacity(roots.len());
    for index in order {
        acquired.push((index, lock_policy(&roots[index], policy_digest)?));
    }
    let expected = format!("{POLICY_LOCK_RECORD}\n{policy_digest}\n").into_bytes();
    let mut unbound = Vec::new();
    for (index, (_, lock)) in acquired.iter_mut().enumerate() {
        let mut contents = Vec::new();
        lock.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "exo_private_policy_lock")?;
        Read::by_ref(&mut lock.file)
            .take(MAX_POLICY_LOCK_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|_| "exo_private_policy_lock")?;
        if contents.len() as u64 > MAX_POLICY_LOCK_BYTES
            || (!contents.is_empty() && contents != expected)
        {
            return Err("exo_private_policy_owner");
        }
        if contents.is_empty() {
            unbound.push(index);
        }
    }
    for index in unbound {
        let lock = &mut acquired[index].1;
        lock.file
            .set_len(0)
            .map_err(|_| "exo_private_policy_lock")?;
        lock.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "exo_private_policy_lock")?;
        lock.file
            .write_all(&expected)
            .map_err(|_| "exo_private_policy_lock")?;
        lock.file
            .sync_all()
            .map_err(|_| "exo_private_policy_lock")?;
    }
    acquired.sort_by_key(|(index, _)| *index);
    Ok(acquired.into_iter().map(|(_, lock)| lock).collect())
}

fn lock_policy(root: &PolicyRoot, policy_digest: &str) -> Result<PolicyLock, &'static str> {
    let name = OsStr::new(".sts2-policy.lock");
    let mut created = false;
    let file = match openat(
        &root.file,
        name,
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(Errno::NOENT) => match openat(
            &root.file,
            name,
            OFlags::RDWR
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::from_bits_retain(PRIVATE_FILE_MODE),
        ) {
            Ok(fd) => {
                created = true;
                File::from(fd)
            }
            Err(Errno::EXIST) => File::from(
                openat(
                    &root.file,
                    name,
                    OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|_| "exo_private_policy_lock")?,
            ),
            Err(_) => return Err("exo_private_policy_lock"),
        },
        Err(_) => return Err("exo_private_policy_lock"),
    };
    if created {
        rustix::fs::fchmod(&file, Mode::from_bits_retain(PRIVATE_FILE_MODE))
            .map_err(|_| "exo_private_policy_lock")?;
    }
    verify_private_file(&file, true)?;
    rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| "exo_private_policy_busy")?;
    Ok(PolicyLock {
        identity: FileIdentity::from_file(&file)?,
        file,
        policy_digest: policy_digest.to_owned(),
    })
}

pub(in crate::exo_private_state) fn verify_policy_root(
    root: &PolicyRoot,
) -> Result<(), &'static str> {
    let reopened = open_policy_root(&root.path, false)?;
    if !root.identity.matches_file(&reopened.file)?
        || !same_file(&root.file, &reopened.file)?
        || root.ancestors != reopened.ancestors
    {
        return Err("exo_private_root_changed");
    }
    Ok(())
}

pub(in crate::exo_private_state) fn verify_policy_lock(
    root: &PolicyRoot,
    lock: &PolicyLock,
) -> Result<(), &'static str> {
    verify_policy_root(root)?;
    let current = openat(
        &root.file,
        ".sts2-policy.lock",
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_policy_lock")?;
    verify_private_file(&current, true)?;
    if !lock.identity.matches_file(&lock.file)?
        || !same_file(&lock.file, &current)?
        || !lock.identity.matches_file(&current)?
    {
        return Err("exo_private_policy_lock_changed");
    }
    let expected = format!("{POLICY_LOCK_RECORD}\n{}\n", lock.policy_digest);
    let mut contents = Vec::new();
    let mut current = current;
    Read::by_ref(&mut current)
        .take(MAX_POLICY_LOCK_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|_| "exo_private_policy_lock")?;
    if contents.len() as u64 > MAX_POLICY_LOCK_BYTES || contents != expected.as_bytes() {
        return Err("exo_private_policy_owner");
    }
    Ok(())
}
