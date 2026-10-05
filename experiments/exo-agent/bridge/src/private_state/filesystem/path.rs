// SPDX-License-Identifier: MIT

use std::path::{Component, Path, PathBuf};

use super::super::{
    FORBIDDEN_ANY_COMPONENTS, FORBIDDEN_FIRST_COMPONENTS, FORBIDDEN_GAME_MARKERS, Paths, Policy,
};

pub(super) fn validate_policy_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return Err("exo_private_policy");
    }
    let components = path.components().filter_map(|component| match component {
        Component::Normal(name) => name.to_str(),
        _ => None,
    });
    let components = components.collect::<Vec<_>>();
    if components.len() < 2
        || components.iter().any(|component| {
            FORBIDDEN_ANY_COMPONENTS
                .iter()
                .any(|forbidden| component.eq_ignore_ascii_case(forbidden))
                || FORBIDDEN_GAME_MARKERS
                    .iter()
                    .any(|marker| component.to_ascii_lowercase().contains(marker))
        })
        || components
            .first()
            .is_some_and(|first| FORBIDDEN_FIRST_COMPONENTS.contains(first))
    {
        return Err("exo_private_policy");
    }
    Ok(())
}

pub(super) fn path_for_kind<'a>(policy: &'a Policy, kind: &str) -> Result<&'a str, &'static str> {
    match kind {
        "state" => Ok(&policy.state_root),
        "cache" => Ok(&policy.cache_root),
        "temp" => Ok(&policy.temp_root),
        _ => Err("exo_private_marker_identity"),
    }
}

pub(super) fn private_path_for_kind<'a>(
    paths: &'a Paths,
    kind: &str,
) -> Result<&'a Path, &'static str> {
    match kind {
        "state" => Ok(&paths.state_root),
        "cache" => Ok(&paths.cache_root),
        "temp" => Ok(&paths.temp_root),
        _ => Err("exo_private_marker_identity"),
    }
}

pub(super) fn attempt_path(root: &str, attempt: &str) -> PathBuf {
    Path::new(root).join(attempt)
}
