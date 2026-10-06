// SPDX-License-Identifier: MIT

use super::{OutputPump, POLL_QUANTUM, PUMP_STOP_BUDGET};
use std::fs::File;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

const OWNED_REAPER_CHILD_TEST: &str = "runtime_support::runtime_v3::game_information_owner::owner_management_tests::entry_tests::child_output::tests::output_child_waits_for_parent_reaper";

#[test]
fn output_pump_stops_within_bound_when_an_inherited_writer_stays_open() {
    let (stdout_read, inherited_stdout_writer) =
        rustix::pipe::pipe().expect("synthetic child output pipe");
    let (stderr_read, stderr_writer) = rustix::pipe::pipe().expect("synthetic child error pipe");
    drop(stderr_writer);
    let mut pump = OutputPump::new(File::from(stdout_read), File::from(stderr_read), 0)
        .expect("bounded output pump");
    let poll_deadline = Instant::now() + Duration::from_secs(1);
    while pump.poll_count() < 2 && Instant::now() < poll_deadline {
        thread::sleep(POLL_QUANTUM);
    }
    assert!(pump.poll_count() >= 2, "pump polled the still-open pipe");

    let started = Instant::now();
    pump.stop()
        .expect("pump stops without waiting for pipe EOF");
    assert!(started.elapsed() < PUMP_STOP_BUDGET);
    assert!(
        !pump.complete(),
        "the still-open inherited writer must remain visible as incomplete capture"
    );
    drop(inherited_stdout_writer);
}

#[test]
fn runtime_child_drop_kills_and_reaps_its_owned_process_within_bound() {
    let child_process = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", OWNED_REAPER_CHILD_TEST, "--nocapture"])
        .env_clear()
        .env("STS2_TEST_OWNED_CHILD_REAPER", "hold")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn owned long-running helper test");
    let child = super::super::child::RuntimeChild::new(child_process)
        .expect("bounded child owner and output pump");
    let reaped = child.reaped_probe();
    let startup_deadline = Instant::now() + Duration::from_secs(1);
    let child_stdout = loop {
        let stdout = child.captured_stdout();
        let text = String::from_utf8_lossy(&stdout);
        if text.contains("owned-child-hold-ready") || Instant::now() >= startup_deadline {
            break stdout;
        }
        thread::sleep(POLL_QUANTUM);
    };
    let child_stdout = String::from_utf8_lossy(&child_stdout);
    assert!(
        child_stdout.lines().any(|line| line == "running 1 test")
            && child_stdout
                .lines()
                .any(|line| line == "owned-child-hold-ready"),
        "the exact held-child helper must be running before cleanup: {child_stdout}"
    );

    let started = Instant::now();
    drop(child);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "owned child cleanup exceeded its bounded reap and pump-stop windows"
    );
    assert!(
        reaped.load(Ordering::Acquire),
        "Drop must observe the owned child exit before returning"
    );
}

#[test]
fn output_child_waits_for_parent_reaper() {
    if std::env::var("STS2_TEST_OWNED_CHILD_REAPER").as_deref() == Ok("hold") {
        use std::io::Write;
        println!("owned-child-hold-ready");
        std::io::stdout()
            .flush()
            .expect("flush owned-child readiness marker");
        thread::sleep(Duration::from_secs(5));
    }
}
