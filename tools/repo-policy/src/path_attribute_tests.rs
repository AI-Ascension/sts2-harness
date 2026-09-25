// SPDX-License-Identifier: MIT

//! `#[path]` resolution tests for the `RUST002` module-reachability rule.
//!
//! Split out of `modules_tests` so both files keep headroom under the
//! `rust_test_preferred` budget, and so the anchoring coverage (#501) sits with
//! the rule's other `#[path]` cases.

use std::collections::BTreeSet;

use super::unreachable;

#[test]
fn path_attribute_owns_its_own_directory() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "#[path = \"sub/foo.rs\"]\nmod foo;\n"),
            ("src/sub/foo.rs", "mod sibling;\n"),
            ("src/sub/sibling.rs", ""),
            ("src/sub/foo/sibling.rs", ""),
        ],
        &["src/lib.rs"],
    );
    // A `#[path]` file looks for children in its own directory, not a stem
    // subdirectory, so the stem-shaped copy is the orphan.
    assert_eq!(
        reported,
        BTreeSet::from(["src/sub/foo/sibling.rs".to_owned()])
    );
}

#[test]
fn cfg_attr_path_reaches_every_branch() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(unix, path = \"a.rs\")]\n\
                 #[cfg_attr(not(unix), path = \"b.rs\")]\nmod gated;\n",
            ),
            ("src/a.rs", ""),
            ("src/b.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(reported.is_empty(), "{reported:?}");
}

/// A `#[path]` on an **inline** module names the directory its children live in;
/// rustc reads no file there at all. `rustc 1.97.1` exits 0 with
/// `src/thread/child.rs` present, and moving it to `src/child.rs` fails `E0583`
/// naming `src/thread/child.rs`, so the directory-valued branch is what reaches
/// the child and the module owns nothing of its own.
#[test]
fn inline_path_attribute_names_its_childrens_directory() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[path = \"thread\"]\nmod m { pub mod child; }\n",
            ),
            ("src/thread/child.rs", ""),
            ("src/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(reported, BTreeSet::from(["src/child.rs".to_owned()]));
}

/// The inline `#[path]` value is relative to the directory of the *file* that
/// carries it when the declaration is unnested — `src/` for `src/x.rs` — not the
/// `src/x/` stem directory its ordinary `mod` children use. rustc reaches
/// `src/thread/child.rs` and ignores a decoy at `src/x/thread/child.rs`.
#[test]
fn inline_path_attribute_is_relative_to_the_carrying_file() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "mod x;\n"),
            (
                "src/x.rs",
                "#[path = \"thread\"]\nmod m { pub mod child; }\n",
            ),
            ("src/thread/child.rs", ""),
            ("src/x/thread/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(
        reported,
        BTreeSet::from(["src/x/thread/child.rs".to_owned()])
    );
}

/// An enclosing inline module is part of the base before the `#[path]` is
/// applied, and the path then supersedes it: rustc reaches
/// `src/x/a/thread/child.rs`, not `src/a/thread/child.rs` or `src/thread/`.
#[test]
fn inline_path_attribute_keeps_its_enclosing_directories() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "mod x;\n"),
            (
                "src/x.rs",
                "pub mod a { #[path = \"thread\"] pub mod b { pub mod child; } }\n",
            ),
            ("src/x/a/thread/child.rs", ""),
            ("src/a/thread/child.rs", ""),
            ("src/thread/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(
        reported,
        BTreeSet::from([
            "src/a/thread/child.rs".to_owned(),
            "src/thread/child.rs".to_owned(),
        ])
    );
}

/// A `#[path]` written inside a module that itself carries one is **anchored**
/// where the outer path pointed, not beside the file carrying the inner
/// declaration. `rustc 1.97.1` reads `src/thread/other.rs` here: a
/// `compile_error!` in `src/other.rs` never fires (exit 0), and removing the live
/// file fails naming it. This is the only shape that reaches `Base::anchored`;
/// forcing that flag to `false` reds this test while the rest of the suite stays
/// green, and it then reports the live file instead of the orphan.
#[test]
fn inline_path_attribute_inside_a_path_module_is_anchored() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[path = \"thread\"]\nmod m { #[path = \"other.rs\"] pub mod n; }\n",
            ),
            ("src/thread/other.rs", ""),
            ("src/other.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(reported, BTreeSet::from(["src/other.rs".to_owned()]));
}

/// The file named by an inline `#[path]` is **not** compiled — a
/// `compile_error!` in it never fires, and rustc's own hint for a missing child
/// is `src/sub/x.rs/child.rs` — so the rule must keep reporting the named file.
/// This is the control that stops the fix from becoming a blanket exemption for
/// any module carrying a `#[path]`.
#[test]
fn an_inline_path_attribute_does_not_reach_the_file_it_names() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[path = \"sub/x.rs\"]\nmod m { pub mod child; }\n",
            ),
            ("src/sub/x.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(reported, BTreeSet::from(["src/sub/x.rs".to_owned()]));
}

/// A `#[path]` on a semicolon module always names a **file**; a directory value
/// is a rustc error (`couldn't read \`src/thread\`: Is a directory`), so
/// `DIR/mod.rs` must not be reached from it. This pins the deliberate omission.
#[test]
fn a_semicolon_path_attribute_does_not_reach_a_directory_body() {
    let reported = unreachable(
        &[
            ("src/lib.rs", "#[path = \"thread\"]\nmod m;\n"),
            ("src/thread/mod.rs", ""),
            ("src/thread/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(
        reported,
        BTreeSet::from([
            "src/thread/child.rs".to_owned(),
            "src/thread/mod.rs".to_owned(),
        ])
    );
}

/// A semicolon `#[path]` nested in a plain inline module resolves inside that
/// module's directory: `rustc 1.97.1` reaches `src/a/x.rs` and never compiles the
/// decoy at `src/x.rs`. This pins the `pending` half of `Base::base`'s condition,
/// which no other test reaches with `anchored` already false.
#[test]
fn a_semicolon_path_attribute_nested_in_an_inline_module_keeps_its_directory() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "pub mod a {\n    #[path = \"x.rs\"]\n    pub mod m;\n}\n",
            ),
            ("src/a/x.rs", ""),
            ("src/x.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert_eq!(reported, BTreeSet::from(["src/x.rs".to_owned()]));
}
