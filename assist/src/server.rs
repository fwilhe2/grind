// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A model behind llama.cpp's server, reached over loopback (`doc/formula-assistant.md` §4.2,
//! A1).
//!
//! The cheapest way to put real models through the benchmark: no inference runtime in this crate,
//! any GGUF model, and `scripts/llama-server.sh` makes the server itself reproducible. It is
//! **not** the shape the assistant ships in — that is A3's decision, between `llama-cpp-2` and
//! candle — which is why this is a plain HTTP/1.1 client over `std::net` rather than a dependency
//! on an HTTP stack.
//!
//! **Loopback only**, refused otherwise: constraint 3 of that document is that nothing leaves the
//! machine, and a prompt carries the document's cells.
//!
//! Candidate 0 is greedy (`temperature: 0`); candidate *i* after it is sampled at
//! [`Server::temperature`] with seed *i*. So a run is reproducible end to end: the same server,
//! model, prompt and count give the same candidates, and asking for five gives the one asked for
//! one first.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Backend, Request};

/// One server.
pub struct Server {
    /// `127.0.0.1:8080`.
    address: String,
    /// The temperature every candidate after the first is sampled at.
    pub temperature: f64,
    /// The longest reply asked for, in tokens. A formula is a line; a model that would write more
    /// is cut off rather than waited for.
    pub max_tokens: u32,
    /// Start the model's reply with `=` and stop it at the end of the line. Part of what is
    /// measured, and on by default: A1's first run found SmolLM3 explaining every formula after
    /// it was told not to, at three tokens a second on the floor's hardware — half a minute a
    /// candidate spent on prose nobody reads, and prose that sometimes came *before* the formula.
    pub prefill: bool,
    /// What the server reported for every request, in order.
    pub timings: Vec<Timing>,
    /// Every raw reply, in order, before [`formula_in`] read a formula out of it.
    pub replies: Vec<String>,
}

/// What llama-server says one request cost.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timing {
    /// Prompt tokens evaluated — the ones the server had not kept from the last request.
    pub prompt_tokens: u64,
    pub prompt_ms: f64,
    pub predicted_tokens: u64,
    pub predicted_ms: f64,
}

impl Server {
    /// A server at `url`, which must be `http://` on a loopback address.
    pub fn new(url: &str) -> Result<Self, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("{url}: only http:// on loopback is spoken"))?;
        let address = rest.trim_end_matches('/');
        let host = match address.rsplit_once(':') {
            Some((host, port)) if port.parse::<u16>().is_ok() => host,
            _ => return Err(format!("{url}: give the port, e.g. http://127.0.0.1:8080")),
        };
        if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
            return Err(format!(
                "{url}: refused — the assistant only talks to this machine (127.0.0.1, localhost, [::1])"
            ));
        }
        Ok(Server {
            address: address.to_owned(),
            temperature: 0.7,
            max_tokens: 120,
            prefill: true,
            timings: Vec::new(),
            replies: Vec::new(),
        })
    }

    /// The model the server has loaded, as it names it.
    pub fn model(&self) -> Result<String, String> {
        let reply = self.http("GET", "/v1/models", None)?;
        reply["data"][0]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("no model in {reply}"))
    }

    fn complete(&mut self, request: &Request, candidate: usize) -> Result<String, String> {
        let temperature = match candidate {
            0 => 0.0,
            _ => self.temperature,
        };
        let mut messages = vec![
            json!({"role": "system", "content": request.prompt.system}),
            json!({"role": "user", "content": request.prompt.user}),
        ];
        if self.prefill {
            // A last message from the assistant is a prefill: the model continues it.
            messages.push(json!({"role": "assistant", "content": "="}));
        }
        let mut body = json!({
            "messages": messages,
            "temperature": temperature,
            "seed": candidate,
            "max_tokens": self.max_tokens,
            // The system part is the same for every request; the server keeps it evaluated.
            "cache_prompt": true,
            // SmolLM3 and OLMo 3 reason aloud unless told not to, and a formula wants no essay.
            "chat_template_kwargs": {"enable_thinking": false},
        });
        if self.prefill {
            body["stop"] = json!(["\n"]);
        }
        let reply = self.http("POST", "/v1/chat/completions", Some(&body))?;
        let timings = &reply["timings"];
        self.timings.push(Timing {
            prompt_tokens: timings["prompt_n"].as_u64().unwrap_or(0),
            prompt_ms: timings["prompt_ms"].as_f64().unwrap_or(0.0),
            predicted_tokens: timings["predicted_n"].as_u64().unwrap_or(0),
            predicted_ms: timings["predicted_ms"].as_f64().unwrap_or(0.0),
        });
        let mut content = reply["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| format!("no reply in {reply}"))?
            .to_owned();
        // Whether a server hands the prefill back with the continuation differs by version.
        if self.prefill && !content.trim_start().starts_with('=') {
            content.insert(0, '=');
        }
        self.replies.push(content.clone());
        Ok(content)
    }

    fn http(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let say = |e: std::io::Error| format!("{}: {e}", self.address);
        let mut stream = TcpStream::connect(&self.address).map_err(say)?;
        // A 7B model on a laptop reading a long prompt the first time takes minutes, not seconds.
        stream
            .set_read_timeout(Some(Duration::from_secs(1800)))
            .map_err(say)?;
        let body = body.map(Value::to_string).unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.address,
            body.len()
        )
        .map_err(say)?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(say)?;
        let text = String::from_utf8_lossy(&raw);
        let (head, payload) = text
            .split_once("\r\n\r\n")
            .ok_or_else(|| format!("{}: not an HTTP reply", self.address))?;
        let status = head.lines().next().unwrap_or_default();
        let payload = match head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            true => dechunk(payload),
            false => payload.to_owned(),
        };
        if !status.contains(" 200 ") {
            return Err(format!("{}{path}: {status}: {payload}", self.address));
        }
        serde_json::from_str(&payload).map_err(|e| format!("{}{path}: {e}", self.address))
    }
}

impl Backend for Server {
    fn propose(&mut self, request: &Request) -> Result<Vec<String>, String> {
        (0..request.candidates)
            .map(|candidate| Ok(formula_in(&self.complete(request, candidate)?)))
            .collect()
    }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let Ok(size) = usize::from_str_radix(size.trim(), 16) else {
            break;
        };
        if size == 0 || after.len() < size {
            break;
        }
        out.push_str(&after[..size]);
        rest = after[size..].trim_start_matches("\r\n");
    }
    out
}

/// The formula in a model's reply. Part of what is measured, so it is deliberately simple and
/// written down: drop a `<think>` block, then take the first line that starts with `=` once code
/// fences and backticks are stripped — or, failing that, the first non-empty line.
pub fn formula_in(reply: &str) -> String {
    let reply = match reply.find("</think>") {
        Some(end) => &reply[end + "</think>".len()..],
        None => reply,
    };
    let lines: Vec<&str> = reply
        .lines()
        .map(|line| line.trim().trim_matches('`').trim())
        .filter(|line| !line.is_empty() && !line.starts_with("```"))
        .collect();
    lines
        .iter()
        .find(|line| line.starts_with('='))
        .or(lines.first())
        .map(|line| (*line).to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_machine_is_spoken_to() {
        assert!(Server::new("http://127.0.0.1:8080").is_ok());
        assert!(Server::new("http://localhost:8080/").is_ok());
        assert!(Server::new("http://[::1]:8080").is_ok());
        for refused in [
            "http://192.168.1.4:8080",
            "http://example.com:8080",
            "https://127.0.0.1:8080",
            "http://127.0.0.1",
            "http://127.0.0.1.example.com:80",
        ] {
            assert!(Server::new(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn a_formula_is_read_out_of_what_a_model_says() {
        assert_eq!(formula_in("=SUM(A1:A3)"), "=SUM(A1:A3)");
        assert_eq!(formula_in("```\n=SUM(A1:A3)\n```"), "=SUM(A1:A3)");
        assert_eq!(formula_in("`=SUM(A1:A3)`"), "=SUM(A1:A3)");
        assert_eq!(
            formula_in("<think>\nhmm\n</think>\nThe answer:\n=SUM(A1:A3)"),
            "=SUM(A1:A3)"
        );
        assert_eq!(formula_in("SUM(A1:A3)"), "SUM(A1:A3)");
        assert_eq!(formula_in(""), "");
    }

    #[test]
    fn a_chunked_body_is_put_back_together() {
        assert_eq!(dechunk("4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"), "Wikipedia");
    }
}
