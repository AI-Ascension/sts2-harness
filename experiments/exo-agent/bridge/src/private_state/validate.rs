// SPDX-License-Identifier: MIT

use std::path::Path;

use rustix::fs::{Mode, OFlags, openat};

use crate::Invocation;

use super::*;

pub(crate) fn set_private_umask_and_validate(invocation: &Invocation) -> Result<(), &'static str> {
    let private = invocation
        .private_state
        .as_ref()
        .ok_or("exo_private_state_missing")?;
    let expected_pid = rustix::process::getpid().as_raw_nonzero().get() as u32;
    let expected_parent = rustix::process::getppid()
        .ok_or("exo_private_process_identity")?
        .as_raw_nonzero()
        .get() as u32;
    let uid = rustix::process::geteuid().as_raw();
    let actual_process = proc_identity(expected_pid)?;
    if private.version != PRIVATE_STATE_VERSION
        || !valid_attempt_id(&private.attempt_id)
        || !valid_digest(&private.config_digest)
        || !valid_digest(&private.policy_digest)
        || private.service_uid != uid
        || private.process.pid != expected_pid
        || private.process.parent_pid != expected_parent
        || private.process.uid != uid
        || private.process.parent_pid != actual_process.parent_pid
        || private.process.process_group != actual_process.process_group
        || private.process.process_group != expected_pid
        || private.process.session != actual_process.session
        || private.process.start_time_ticks != actual_process.start_time_ticks
        || private.process.boot_id != read_boot_id()?
        || !invocation.state_root.is_absolute()
    {
        return Err("exo_private_process_identity");
    }
    private.policy.validate()?;
    if policy_digest(&private.policy) != private.policy_digest
        || invocation.state_root != private.paths.state_root
        || private.paths.state_root != attempt_path(&private.policy.state_root, &private.attempt_id)
        || private.paths.cache_root != attempt_path(&private.policy.cache_root, &private.attempt_id)
        || private.paths.temp_root != attempt_path(&private.policy.temp_root, &private.attempt_id)
        || private.paths.config_root != private.paths.cache_root.join("config")
    {
        return Err("exo_private_policy_identity");
    }

    let roots = [
        ("state", private.policy.state_root.as_str()),
        ("cache", private.policy.cache_root.as_str()),
        ("temp", private.policy.temp_root.as_str()),
    ];
    let mut opened = Vec::with_capacity(roots.len());
    for (kind, path) in roots {
        opened.push((kind, open_private_root(Path::new(path))?));
    }
    reject_root_aliases(&opened)?;
    for (_, root) in &opened {
        verify_policy_lock(root, &private.policy_digest)?;
    }

    if std::env::var_os("XDG_CONFIG_HOME").as_deref() != Some(private.paths.config_root.as_os_str())
        || std::env::var_os("XDG_CACHE_HOME").as_deref()
            != Some(private.paths.cache_root.as_os_str())
        || std::env::var_os("TMPDIR").as_deref() != Some(private.paths.temp_root.as_os_str())
        || std::env::var_os("EXO_LITELLM_PRICES_PATH").as_deref()
            != Some(private.paths.cache_root.join("no-prices.json").as_os_str())
    {
        return Err("exo_private_environment");
    }

    let attempt_id = std::ffi::OsStr::new(&private.attempt_id);
    let mut attempt_ids = Vec::with_capacity(opened.len());
    for (_, root) in &opened {
        let attempt = openat(
            &root.file,
            attempt_id,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| "exo_private_attempt")?;
        verify_directory(&attempt, uid)?;
        attempt_ids.push(file_identity(&attempt)?);
    }
    let expected_proofs = opened
        .iter()
        .zip(&attempt_ids)
        .zip(["state", "cache", "temp"])
        .map(|(((_, root), attempt), kind)| RootProof {
            kind: kind.to_owned(),
            base_identity: root.identity.clone(),
            attempt_identity: attempt.clone(),
        })
        .collect::<Vec<_>>();

    let mut markers = Vec::with_capacity(opened.len());
    for (kind, root) in &opened {
        let attempt = openat(
            &root.file,
            attempt_id,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| "exo_private_attempt")?;
        verify_directory(&attempt, uid)?;
        let attempt_identity = file_identity(&attempt)?;
        let expected_path =
            attempt_path(path_for_kind(&private.policy, kind)?, &private.attempt_id);
        if expected_path != private_path_for_kind(&private.paths, kind)? {
            return Err("exo_private_attempt");
        }
        let marker = read_marker(&attempt)?;
        verify_marker(
            &marker,
            private,
            kind,
            &root.identity,
            &attempt_identity,
            &expected_proofs,
        )?;
        verify_attempt_lock(&attempt)?;
        markers.push(marker);
    }
    if markers
        .iter()
        .skip(1)
        .any(|marker| !same_shared_marker(&markers[0], marker))
    {
        return Err("exo_private_marker_mismatch");
    }
    let cache_attempt = openat(
        &opened[1].1.file,
        std::ffi::OsStr::new(&private.attempt_id),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_attempt")?;
    let config = openat(
        &cache_attempt,
        "config",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_config_root")?;
    verify_directory(&config, uid)?;
    if file_identity(&config)? != file_identity_path(&private.paths.config_root)? {
        return Err("exo_private_config_root");
    }

    // This is a dedicated executor process. Setting umask is process-wide; it happens before
    // BasicExoHarness, SQLite/WAL files, secret storage, or provider binding can create files.
    let _previous = rustix::process::umask(Mode::from_bits_retain(0o077));
    Ok(())
}
