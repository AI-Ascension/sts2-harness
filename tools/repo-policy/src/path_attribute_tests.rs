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

/// The deliberate over-approximation, pinned so a future tightening is a red
/// test rather than a silent behaviour change: when a plain same-named sibling
/// exists beside an **exhaustive** `#[cfg_attr]` pair, that sibling is dead
/// under every configuration but stays silent.
///
/// Measured (`rustc 1.97.1`, per-file markers): with `unix` / `not(unix)` on one
/// `mod snap;`, `src/snap_unix.rs` fires and `src/snap_other.rs` does not, so
/// `src/snap.rs` is read by no configuration. The same rule covers the inline
/// block form: `src/m/child.rs` carries no firing marker while
/// `src/altu/child.rs` does. A scan that dropped the name branch to report these
/// would instead report a live file on `a_non_exhaustive_pair_keeps_the_name_branch`.
#[test]
fn cfg_attr_path_pairs() {
    let semicolon = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(unix, path = \"snap_unix.rs\")]\n\
                 #[cfg_attr(not(unix), path = \"snap_other.rs\")]\n\
                 mod snap;\n",
            ),
            ("src/snap_unix.rs", ""),
            ("src/snap_other.rs", ""),
            ("src/snap.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(semicolon.is_empty(), "{semicolon:?}");

    let inline = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(unix, path = \"altu\")]\n\
                 #[cfg_attr(not(unix), path = \"altw\")]\n\
                 mod m { pub mod child; }\n",
            ),
            ("src/altu/child.rs", ""),
            ("src/altw/child.rs", ""),
            ("src/m/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(inline.is_empty(), "{inline:?}");
}

/// The counterexample that decides the direction above: a pair whose gates are
/// **not** exhaustive leaves the plain name live. With
/// `#[cfg_attr(feature = "x", path = "a.rs")]` and
/// `#[cfg_attr(feature = "y", path = "b.rs")]` on one `mod m;`, a per-file
/// marker fires for `src/m.rs` under the default build, for `src/a.rs` with
/// `--cfg 'feature="x"'`, and for `src/b.rs` with `--cfg 'feature="y"'` — all
/// three are real, so nothing may be reported. Keying the name branch off the
/// *number* of `#[path]` attributes reports `src/m.rs` here, which is why the
/// branch is kept rather than narrowed.
#[test]
fn a_non_exhaustive_pair_keeps_the_name_branch() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(feature = \"x\", path = \"a.rs\")]\n\
                 #[cfg_attr(feature = \"y\", path = \"b.rs\")]\n\
                 mod m;\n",
            ),
            ("src/m.rs", ""),
            ("src/a.rs", ""),
            ("src/b.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(reported.is_empty(), "{reported:?}");
}

/// A **pair** of mutually exclusive `#[cfg_attr]`s is the case where the
/// branch-union rule costs precision, so the cost is pinned here rather than
/// left implicit. With `#[cfg_attr(unix, path = "a.rs")]` and
/// `#[cfg_attr(not(unix), path = "b.rs")]` on one `mod imp;`, a host build
/// reads `src/a.rs` only (`src/b.rs` and `src/imp.rs` carry no marker that
/// fires), yet the scan still credits the ordinary `NAME.rs` lookup because a
/// gate it cannot evaluate may have left the declaration without any
/// `#[path]` at all. `src/imp.rs` is therefore silent, not reported. This
/// mirrors the merged `sts2-gateway` copy at tree `f9ba7ff9`, which is also
/// silent here; it is a deliberate both-branches over-approximation, and losing
/// the name branch instead would report a live file.
#[test]
fn a_pair_of_mutually_exclusive_cfg_attr_paths_still_credits_the_name_branch() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(unix, path = \"a.rs\")]\n\
                 #[cfg_attr(not(unix), path = \"b.rs\")]\n\
                 mod imp;\n",
            ),
            ("src/a.rs", ""),
            ("src/b.rs", ""),
            ("src/imp.rs", ""),
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

/// A `#[cfg_attr]`-gated `#[path]` on an **inline** module replaces the
/// name-based lookup only on the branch that takes it, so the other branch's
/// directory is still reached. `rustc 1.97.1` under the default build reads
/// `src/m/child.rs` (a marker there fails the build; the same marker in
/// `src/alt/child.rs` is never read), so reporting the live file is the
/// delete-a-live-file direction. Before this was handled the gate reported
/// `src/m/child.rs` as unreachable.
#[test]
fn a_cfg_attr_path_on_an_inline_block_keeps_the_name_based_branch() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(feature = \"x\", path = \"alt\")]\nmod m { pub mod child; }\n",
            ),
            ("src/m/child.rs", ""),
            ("src/alt/child.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(reported.is_empty(), "{reported:?}");
}

/// The same rule for a **semicolon** `mod`: `#[cfg_attr(feature = "x",
/// path = "alt.rs")] mod m;` reads `src/m.rs` under the default build and
/// `src/alt.rs` with the gate taken, so both branches must be credited and
/// neither is an orphan.
#[test]
fn a_cfg_attr_path_on_a_semicolon_mod_reaches_every_branch() {
    let reported = unreachable(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(feature = \"x\", path = \"alt.rs\")]\nmod m;\n",
            ),
            ("src/m.rs", ""),
            ("src/alt.rs", ""),
        ],
        &["src/lib.rs"],
    );
    assert!(reported.is_empty(), "{reported:?}");
}
