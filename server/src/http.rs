// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A request and a response as plain values, so [`crate::Server::handle`] is a function a test
//! calls with no socket in sight.

use serde::Serialize;

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    /// The path as sent, still percent-encoded, without the query.
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// `target` is what the request line carries: a path and maybe `?query`.
    #[must_use]
    pub fn new(method: &str, target: &str) -> Self {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        Request {
            method: method.to_owned(),
            path: path.to_owned(),
            query: query.to_owned(),
            ..Request::default()
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
    }

    /// A query parameter, decoded.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<String> {
        self.query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(n, _)| *n == name)
            .and_then(|(_, v)| percent_decode(&v.replace('+', " ")))
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    #[must_use]
    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Response {
            status,
            headers: vec![("Content-Type".to_owned(), content_type.to_owned())],
            body,
        }
    }

    pub fn json(status: u16, value: &impl Serialize) -> Self {
        let body = serde_json::to_vec(value).unwrap_or_default();
        Response::bytes(status, "application/json", body)
    }

    /// `{"error": message}`, the one shape every refusal has.
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        Response::json(status, &serde_json::json!({ "error": message.into() }))
    }

    #[must_use]
    pub fn empty(status: u16) -> Self {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// `%XX` decoded as UTF-8; `None` for a broken escape or bytes that are not UTF-8.
#[must_use]
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_in_the_query_is_decoded() {
        let request = Request::new("POST", "/api/v1/docs?name=Haushalt%202026.fods&x=1");
        assert_eq!(request.param("name").as_deref(), Some("Haushalt 2026.fods"));
        assert_eq!(percent_decode("%C3%A4"), Some("ä".to_owned()));
        assert_eq!(percent_decode("%4"), None);
        assert_eq!(percent_decode("%FF"), None);
    }
}
