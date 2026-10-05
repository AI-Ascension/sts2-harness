// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn isolated_subreaper_child() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("escape")) {
        return Ok(());
    }
    let scratch = Scratch::new("escaped")?;
    let mut run =
        GuardedRun::create(&scratch.policy(), &crate::sha256_hex(b"config")).map_err(io_error)?;
    let held_path = run.paths().state_root.join("held-open-state");
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&held_path)?;
    let scope = BridgeChildScope::enable().map_err(io_error)?;
    run.begin_spawn().map_err(io_error)?;

    let mut child = shell_executor(&held_path).spawn()?;
    let pid = child.id();
    let _state = run.record_child(pid).map_err(io_error)?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("executor stdin was not piped"))?;
    input.write_all(b"start\n")?;
    drop(input);
    let status = child.wait()?;
    assert!(status.success(), "the executor leader must exit normally");

    assert!(held_path.exists(), "live escaped child state is retained");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while fs::metadata(&held_path)?.len() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        fs::metadata(&held_path)?.len() > 0,
        "the escaped session child must have written the held state"
    );
    let clean = run.finish_child(&scope).map_err(io_error)?;
    assert!(!clean, "a killed escaped descendant must withhold success");
    run.finish().map_err(io_error)?;
    assert!(!held_path.exists(), "cleanup follows descendant drainage");
    drop(run);
    fs::remove_dir_all(&scratch.0)?;
    Ok(())
}

#[test]
fn isolated_clean_child() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("clean")) {
        return Ok(());
    }
    let scratch = Scratch::new("clean")?;
    let mut run = GuardedRun::create(&scratch.policy(), &crate::sha256_hex(b"clean-config"))
        .map_err(io_error)?;
    let paths = run.paths().clone();
    let scope = BridgeChildScope::enable().map_err(io_error)?;
    run.begin_spawn().map_err(io_error)?;
    let mut child = Command::new("/bin/sh")
        .process_group(0)
        .arg("-c")
        .arg("read trigger; exit 0")
        .stdin(Stdio::piped())
        .spawn()?;
    let _state = run.record_child(child.id()).map_err(io_error)?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("clean child stdin was not piped"))?;
    input.write_all(b"start\n")?;
    drop(input);
    let status = child.wait()?;
    assert!(status.success());
    assert!(run.finish_child(&scope).map_err(io_error)?);
    run.finish().map_err(io_error)?;
    drop(run);
    for attempt in [paths.state_root, paths.cache_root, paths.temp_root] {
        assert!(!attempt.exists(), "successful cleanup removes {attempt:?}");
    }
    fs::remove_dir_all(&scratch.0)?;
    Ok(())
}

#[test]
fn isolated_preexisting_live_child_is_refused_without_reaping() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("live-child")) {
        return Ok(());
    }
    let child = std::thread::spawn(|| {
        Command::new("/bin/sh")
            .arg("-c")
            .arg("read trigger; exit 0")
            .stdin(Stdio::piped())
            .spawn()
    })
    .join()
    .map_err(|_| io::Error::other("child-spawn thread panicked"))??;
    assert_eq!(
        BridgeChildScope::enable().err(),
        Some("exo_private_preexisting_child")
    );
    let mut child = child;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("live child stdin was not piped"))?;
    input.write_all(b"start\n")?;
    drop(input);
    assert!(
        child.wait()?.success(),
        "the refused live child remains owned"
    );
    Ok(())
}

#[test]
fn isolated_preexisting_zombie_child_is_refused_without_reaping() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("zombie-child")) {
        return Ok(());
    }
    let child = std::thread::spawn(|| Command::new("/bin/sh").arg("-c").arg("exit 0").spawn())
        .join()
        .map_err(|_| io::Error::other("child-spawn thread panicked"))??;
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let state = stat
            .rfind(')')
            .and_then(|index| stat.get(index + 1..))
            .and_then(|suffix| suffix.split_whitespace().next())
            .and_then(|field| field.chars().next());
        if state == Some('Z') {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::other("preexisting child did not become a zombie").into());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        BridgeChildScope::enable().err(),
        Some("exo_private_preexisting_child")
    );
    assert!(
        child.wait()?.success(),
        "the refused zombie remains waitable"
    );
    Ok(())
}

fn shell_executor(held_path: &Path) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .process_group(0)
        .arg("-c")
        .arg("read trigger; setsid /bin/sh -c 'exec 3>>\"$1\"; while :; do printf x >&3; sleep 0.01; done' child \"$1\" >/dev/null 2>&1 & exit 0")
        .arg("exo-private-test")
        .arg(held_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}
