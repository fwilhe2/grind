// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! LanguageTool's requests (`doc/grammar.md`) through the system's `curl`, which every Mac
//! ships: its TLS is the system's, and nothing is compiled in — the HTTP client the crate brings
//! would not type-check for this target from Linux, as `macos.yml` does.

/// `POST` `form`, form-encoded, to `server`'s `/v2/check`; the body of a 200, or why not.
pub fn post(
    server: &grind_languagetool::Server,
    form: &[(String, String)],
) -> Result<String, String> {
    use std::io::Write;
    let body: String = form
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let mut child = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--max-time",
            "15",
            "--header",
            "Content-Type: application/x-www-form-urlencoded",
            "--data-binary",
            "@-",
            // The status after the body, on a line of its own.
            "--write-out",
            "\n%{http_code}",
        ])
        .arg(format!("{}/v2/check", server.base()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl could not be run: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(body.as_bytes())
            .map_err(|e| format!("curl took no request: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("curl did not finish: {e}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{}: {}", server.host(), said.trim()));
    }
    let answer = String::from_utf8_lossy(&output.stdout);
    let (body, status) = answer.rsplit_once('\n').unwrap_or(("", &answer));
    match status.trim() {
        "200" => Ok(body.to_owned()),
        status => Err(format!("{} answered {status}", server.host())),
    }
}

/// `application/x-www-form-urlencoded`'s escaping: letters, digits and `-_.~` as they are, a
/// space as `+`, every other byte as `%XX`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    /// A one-shot server answering `status` and `body`, handing back the request body it got.
    fn mock(status: u16, body: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
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
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (url, rx)
    }

    #[test]
    fn a_form_is_posted_and_the_answer_read() {
        let (url, request) = mock(200, r#"{"matches":[]}"#);
        let server = grind_languagetool::Server::SelfHosted(url);
        let form = [
            ("text".to_owned(), "Grüße & so".to_owned()),
            ("language".to_owned(), "de-DE".to_owned()),
        ];
        assert_eq!(post(&server, &form).as_deref(), Ok(r#"{"matches":[]}"#));
        assert_eq!(
            request.recv().unwrap(),
            "text=Gr%C3%BC%C3%9Fe+%26+so&language=de-DE"
        );
    }

    #[test]
    fn a_failure_says_why() {
        let (url, _request) = mock(500, "oops");
        let server = grind_languagetool::Server::SelfHosted(url);
        let error = post(&server, &[]).unwrap_err();
        assert!(error.contains("500"), "{error}");
        let nobody = grind_languagetool::Server::SelfHosted("http://127.0.0.1:9".into());
        assert!(post(&nobody, &[]).is_err());
    }

    /// The real server, where `GRIND_LANGUAGETOOL_URL` names one.
    #[test]
    fn the_real_server_answers() {
        let Ok(url) = std::env::var("GRIND_LANGUAGETOOL_URL") else {
            return;
        };
        let server = grind_languagetool::Server::SelfHosted(url);
        let form = [
            ("text".to_owned(), "He go home.".to_owned()),
            ("language".to_owned(), "en-US".to_owned()),
        ];
        let answer = post(&server, &form).expect("an answer");
        assert!(answer.contains("HE_VERB_AGR"), "{answer}");
    }
}
