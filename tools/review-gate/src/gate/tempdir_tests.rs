// SPDX-License-Identifier: MIT

//! #713: the stub helpers must not accumulate directories in `$TMPDIR`.

#[cfg(unix)]
use super::deadline::grandchild_gh;
use super::runner::fake_gh;
use std::error::Error;

/// #713: the stub helpers must leave nothing behind once their guard is gone.
///
/// The acceptance criterion asks that this be shown to fail when the cleanup is removed, so the
/// test is shaped to make that removal obvious rather than incidental: it records the directory
/// the helper actually made, keeps the guard alive long enough to prove the stub is really there,
/// then drops the guard and asserts the directory is gone. Deleting `TempDir`'s `Drop` impl
/// leaves every earlier assertion in the suite passing and fails exactly this one, which is the
/// failure a reader of #713 would recognize.
#[test]
fn fake_gh_leaves_no_directory_behind() -> Result<(), Box<dyn Error>> {
    let survivor = {
        let (_guard, path) = fake_gh("{}", "", 0)?;
        let directory = path
            .parent()
            .ok_or("the stub path must have a parent directory")?
            .to_path_buf();
        // While the guard is alive the stub must really be present, or the assertion after the
        // drop would be satisfied by a helper that never created anything in the first place.
        assert!(
            directory.is_dir(),
            "the stub directory {directory:?} must exist while its guard is alive"
        );
        assert!(
            path.is_file(),
            "the stub executable {path:?} must exist while its guard is alive"
        );
        // `_guard` drops here, at the end of the block, and takes the directory with it.
        directory
    };
    assert!(
        !survivor.exists(),
        "the stub directory {survivor:?} survived its guard, so every test run leaks one"
    );
    Ok(())
}

/// #719: `grandchild_gh` must not accumulate directories either.
///
/// The test above covers `fake_gh` only, and that narrowness is exactly how #719 happened:
/// #718 added a second leaking helper, the existing property still passed, and the leak shipped
/// on `main`. Each helper that creates a stub directory therefore needs its own assertion here,
/// because a property that is only exercised at one call site cannot notice a second one.
///
/// Unix-only because `grandchild_gh` is; there is no Windows stub directory to leak.
#[cfg(unix)]
#[test]
fn grandchild_gh_leaves_no_directory_behind() -> Result<(), Box<dyn Error>> {
    let survivor = {
        let (_guard, path, pid_file) = grandchild_gh()?;
        let directory = path
            .parent()
            .ok_or("the stub path must have a parent directory")?
            .to_path_buf();
        // Same reason as the test above: the assertion after the drop would be satisfied by a
        // helper that never created anything, so the directory and both files must be present
        // while the guard is alive.
        assert!(
            directory.is_dir(),
            "the stub directory {directory:?} must exist while its guard is alive"
        );
        assert!(
            path.is_file(),
            "the stub executable {path:?} must exist while its guard is alive"
        );
        // The pid file is written by the stub at run time, not by `grandchild_gh`, so it is
        // absent here and its existence is what the deadline test asserts. Only the directory
        // and the executable are created eagerly.
        assert!(
            !pid_file.exists(),
            "the pid file {pid_file:?} is written by the stub at run time, not before"
        );
        // `_guard` drops here and takes the directory with it.
        directory
    };
    assert!(
        !survivor.exists(),
        "the stub directory {survivor:?} survived its guard, so every test run leaks one"
    );
    Ok(())
}
