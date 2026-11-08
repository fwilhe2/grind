// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind serve` as a process: it stops on SIGTERM by its own handler, which is what makes it
//! stop as a container's PID 1, where a signal with no handler is ignored.

#![cfg(all(unix, feature = "server"))]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn sigterm_stops_it_cleanly() {
    let dir = std::env::temp_dir().join(format!("grind-serve-sigterm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_grind"))
        .args(["serve", dir.to_str().unwrap(), "--port", "0"])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = String::new();
    BufReader::new(child.stderr.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    assert!(first.contains("http://127.0.0.1:"), "{first}");

    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "grind serve ignored SIGTERM");
        std::thread::sleep(Duration::from_millis(20));
    };
    // Killed by the signal's default action would be no exit code at all.
    assert_eq!(status.code(), Some(0), "{status}");
    let _ = std::fs::remove_dir_all(&dir);
}
