// SPDX-License-Identifier: MIT

//! Repository-relative path arithmetic shared by the `RUST002` scan.

/// The directory part of a repository-relative path (empty for a top-level file).
pub(crate) fn directory(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_owned(),
        None => String::new(),
    }
}

pub(crate) fn join(dir: &str, relative: &str) -> String {
    if dir.is_empty() {
        normalise(relative)
    } else {
        normalise(&format!("{dir}/{relative}"))
    }
}

pub(crate) fn join_all(dir: &str, parts: &[String]) -> String {
    let mut joined = dir.to_owned();
    for part in parts {
        joined = join(&joined, part);
    }
    joined
}

pub(crate) fn normalise(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}
