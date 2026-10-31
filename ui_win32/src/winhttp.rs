// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One `POST` through WinHTTP, Windows' own HTTP stack: LanguageTool's requests
//! (`doc/grammar.md`), with the system's certificates and proxy and nothing compiled in.

use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle WinHTTP gave and nothing else closes.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn opened(handle: *mut core::ffi::c_void, what: &str) -> Result<Handle, String> {
    match handle.is_null() {
        true => Err(format!("WinHTTP could not {what}")),
        false => Ok(Handle(handle)),
    }
}

/// `POST` `form`, form-encoded, to `server`'s `/v2/check`; the body of a 200, or why not.
pub fn post(
    server: &grind_languagetool::Server,
    form: &[(String, String)],
) -> Result<String, String> {
    let url: Vec<u16> = format!("{}/v2/check", server.base())
        .encode_utf16()
        .collect();
    let mut host = [0u16; 256];
    let mut path = [0u16; 2048];
    let mut parts = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        lpszHostName: windows::core::PWSTR(host.as_mut_ptr()),
        dwHostNameLength: host.len() as u32,
        lpszUrlPath: windows::core::PWSTR(path.as_mut_ptr()),
        dwUrlPathLength: path.len() as u32,
        ..Default::default()
    };
    let body: String = form
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let headers: Vec<u16> = "Content-Type: application/x-www-form-urlencoded\r\n"
        .encode_utf16()
        .collect();
    // SAFETY: every buffer handed to WinHTTP outlives the call that reads it, and every handle
    // is closed by its `Handle` on the way out.
    unsafe {
        WinHttpCrackUrl(&url, 0, &mut parts).map_err(|e| format!("not a URL: {e}"))?;
        let session = opened(
            WinHttpOpen(
                w!("grind"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "start",
        )?;
        let _ = WinHttpSetTimeouts(session.0, 5_000, 5_000, 10_000, 10_000);
        let host = PCWSTR(host.as_ptr());
        let connection = opened(
            WinHttpConnect(session.0, host, parts.nPort, 0),
            &format!("reach {}", server.host()),
        )?;
        let flags = match parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS {
            true => WINHTTP_FLAG_SECURE,
            false => WINHTTP_OPEN_REQUEST_FLAGS(0),
        };
        let request = opened(
            WinHttpOpenRequest(
                connection.0,
                w!("POST"),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                flags,
            ),
            "make the request",
        )?;
        let sent = body.len() as u32;
        WinHttpSendRequest(
            request.0,
            Some(&headers),
            Some(body.as_ptr().cast()),
            sent,
            sent,
            0,
        )
        .map_err(|e| format!("{} could not be reached: {e}", server.host()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("{} did not answer: {e}", server.host()))?;
        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("no status from {}: {e}", server.host()))?;
        let mut answer = Vec::new();
        loop {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut available)
                .map_err(|e| format!("reading {}: {e}", server.host()))?;
            if available == 0 {
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), available, &mut read)
                .map_err(|e| format!("reading {}: {e}", server.host()))?;
            answer.extend_from_slice(&chunk[..read as usize]);
        }
        let answer = String::from_utf8_lossy(&answer).into_owned();
        match status {
            200 => Ok(answer),
            _ => Err(format!("{} answered {status}", server.host())),
        }
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

    /// A one-shot server answering `body`, handing back the request body it got.
    fn mock(body: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
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
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (url, rx)
    }

    #[test]
    fn a_form_is_posted_and_the_answer_read() {
        let (url, request) = mock(r#"{"matches":[]}"#);
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
        let nobody = grind_languagetool::Server::SelfHosted("http://127.0.0.1:9".into());
        assert!(post(&nobody, &form).is_err());
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
