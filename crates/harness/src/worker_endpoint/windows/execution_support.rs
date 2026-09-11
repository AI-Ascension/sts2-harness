// SPDX-License-Identifier: MIT

#[derive(Clone)]
struct ApprovedExecutable {
    image: sts2_harness_windows_boundary::VerifiedExecutable,
}

impl ApprovedExecutable {
    fn command_path(&self) -> std::path::PathBuf {
        self.image.command_path().to_path_buf()
    }

    fn verify_before_launch(&self) -> Result<(), String> {
        self.image.verify_current()
    }
}

fn verify_executable(
    path: &std::path::Path,
    expected: Option<&str>,
) -> Result<ApprovedExecutable, String> {
    let image = sts2_harness_windows_boundary::VerifiedExecutable::open(path, expected)?;
    Ok(ApprovedExecutable { image })
}
