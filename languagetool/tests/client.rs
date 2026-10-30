// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The client against a server: a mock that answers what LanguageTool would, and the real one
//! where `GRIND_LANGUAGETOOL_URL` names it (`scripts/languagetool.sh` starts the container).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::ops::Range;
use std::sync::mpsc;

use grind_core::grammar::{Checker, Paragraph};
use grind_languagetool::{LanguageTool, Server};

/// A one-shot HTTP server answering `status` and `body`, handing back the request body it got.
fn mock(status: u16, body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
        }
        let mut request = vec![0; length];
        reader.read_exact(&mut request).unwrap();
        tx.send(String::from_utf8(request).unwrap()).unwrap();
        let mut stream = stream;
        write!(
            stream,
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    (url, rx)
}

const ANSWER: &str = r#"{"matches":[{"message":"Agreement","offset":3,"length":2,
  "replacements":[{"value":"goes"}],
  "rule":{"id":"HE_VERB_AGR","description":"d","issueType":"grammar",
          "category":{"id":"GRAMMAR","name":"Grammar"}}}]}"#;

fn paragraph<'a>(text: &'a str, skip: &'a [Range<usize>]) -> Paragraph<'a> {
    Paragraph {
        text,
        skip,
        language: Some("en-US"),
    }
}

#[test]
fn a_self_hosted_server_is_asked_and_answers_per_paragraph() {
    let (url, request) = mock(200, ANSWER);
    let checker = LanguageTool::new(Server::SelfHosted(url), false);
    let none = Vec::new();
    let found = checker.check_all(&[paragraph("He go home and it is fine.", &none)]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].len(), 1);
    assert_eq!(found[0][0].rule, "lt:HE_VERB_AGR");
    assert_eq!(found[0][0].range, 3..5);
    let sent = request.recv().unwrap();
    assert!(sent.contains("disabledCategories=TYPOS"), "{sent}");
    assert!(sent.contains("language=en-US"), "{sent}");
    assert_eq!(checker.error(), None);
}

#[test]
fn a_server_that_fails_finds_nothing_and_says_why() {
    let (url, _request) = mock(500, "oops");
    let checker = LanguageTool::new(Server::SelfHosted(url), false);
    let none = Vec::new();
    let found = checker.check_all(&[paragraph("He go home and it is fine.", &none)]);
    assert_eq!(found, [Vec::new()]);
    let error = checker.error().expect("an error");
    assert!(error.contains("500"), "{error}");
    let nobody = LanguageTool::new(Server::SelfHosted("http://127.0.0.1:9".into()), false);
    nobody.check(&paragraph("He go home and it is fine.", &none));
    assert!(nobody.error().is_some());
}

#[test]
fn where_the_text_goes_is_said_in_words() {
    assert_eq!(
        Server::SelfHosted("http://localhost:8010".into()).describe(),
        "checked on this computer"
    );
    assert_eq!(
        Server::SelfHosted("https://lt.example.org".into()).describe(),
        "your text is sent to lt.example.org"
    );
    assert_eq!(
        Server::Public.describe(),
        "your text is sent to api.languagetool.org"
    );
}

/// The real server, where one is running: an agreement error found, in characters.
#[test]
fn the_real_server_finds_what_it_finds() {
    let Ok(url) = std::env::var("GRIND_LANGUAGETOOL_URL") else {
        eprintln!("skipped: GRIND_LANGUAGETOOL_URL names no LanguageTool server");
        return;
    };
    let checker = LanguageTool::new(Server::SelfHosted(url), false);
    let none = Vec::new();
    let found = checker.check_all(&[
        paragraph("😀 He go home and it is fine.", &none),
        paragraph("She go out, and it is the end.", &none),
    ]);
    assert_eq!(checker.error(), None);
    let rules: Vec<Vec<(String, Range<usize>)>> = found
        .iter()
        .map(|f| {
            f.iter()
                .map(|f| (f.rule.clone(), f.range.clone()))
                .collect()
        })
        .collect();
    assert!(
        rules[0].contains(&("lt:HE_VERB_AGR".to_owned(), 5..7)),
        "{rules:?}"
    );
    assert!(rules[1].iter().any(|(_, r)| *r == (4..6)), "{rules:?}");
}
