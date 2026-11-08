// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The server on a real socket: loopback, and an answer over it.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;

#[test]
fn it_listens_on_loopback_and_answers() {
    let dir = std::env::temp_dir().join(format!("grind-server-listen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (tx, rx) = mpsc::channel();
    let options = grind_server::Options {
        dir: dir.clone(),
        port: 0,
        page: None,
        hosts: Vec::new(),
    };
    std::thread::spawn(move || grind_server::serve(options, |port| tx.send(port).unwrap()));
    let port = rx.recv().unwrap();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    assert!(stream.local_addr().unwrap().ip().is_loopback());
    write!(
        stream,
        "GET /api/v1/hello HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("\"protocol\":1"), "{reply}");
    let _ = std::fs::remove_dir_all(&dir);
}
