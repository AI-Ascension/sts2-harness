// SPDX-License-Identifier: MIT

//! Offline packaging and non-inferencing inspection of operator-reviewed Exo inputs.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod exo_package;

fn main() {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let result = exo_package::execute(std::env::args().skip(1).collect());
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    let result: Result<(), &'static str> = Err("exo_package_platform_unsupported");
    if let Err(code) = result {
        eprintln!("{code}");
        std::process::exit(1);
    }
}
