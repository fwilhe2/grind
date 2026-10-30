// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Asking a server, over HTTP, and keeping within what it allows.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use grind_core::grammar::{Checker, Finding, Paragraph};

use crate::{Options, Server, batches};

/// A grammar checker that asks a LanguageTool server. A request that fails finds nothing and
/// leaves its reason in [`LanguageTool::error`]; it never blocks for more than the timeout.
pub struct LanguageTool {
    server: Server,
    options: Options,
    agent: ureq::Agent,
    error: Mutex<Option<String>>,
    /// When each recent request went, and how many characters it carried.
    sent: Mutex<VecDeque<(Instant, usize)>>,
}

impl LanguageTool {
    /// A checker asking `server`, with its style rules when `picky`.
    pub fn new(server: Server, picky: bool) -> Self {
        // The system's certificates; one it cannot read is left out rather than failing all.
        let roots: Vec<ureq::tls::Certificate<'static>> = rustls_native_certs::load_native_certs()
            .certs
            .into_iter()
            .map(|der| ureq::tls::Certificate::from_der(der.as_ref()).to_owned())
            .collect();
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::new_with_certs(&roots))
            .build();
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .tls_config(tls)
            .build()
            .into();
        LanguageTool {
            options: Options {
                picky,
                disabled_rules: Vec::new(),
                max_chars: server.max_chars(),
            },
            server,
            agent,
            error: Mutex::new(None),
            sent: Mutex::new(VecDeque::new()),
        }
    }

    /// Ignore these LanguageTool rules, by their own ids.
    pub fn ignoring(mut self, rules: Vec<String>) -> Self {
        self.options.disabled_rules = rules;
        self
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Why the last request found nothing, or `None` when it was answered.
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    /// Wait until a request of `chars` characters is within the server's limits a minute.
    fn throttle(&self, chars: usize) {
        let Some((requests, characters)) = self.server.per_minute() else {
            return;
        };
        let minute = Duration::from_secs(60);
        loop {
            let mut sent = self.sent.lock().unwrap();
            let now = Instant::now();
            while sent.front().is_some_and(|(at, _)| now - *at > minute) {
                sent.pop_front();
            }
            let used: usize = sent.iter().map(|(_, c)| c).sum();
            if sent.len() < requests && used + chars <= characters || sent.is_empty() {
                sent.push_back((now, chars));
                return;
            }
            let wait = minute.saturating_sub(now - sent.front().map_or(now, |(at, _)| *at));
            drop(sent);
            std::thread::sleep(wait.min(Duration::from_secs(5)));
        }
    }

    fn post(&self, form: &[(String, String)]) -> Result<String, String> {
        let url = format!("{}/v2/check", self.server.base());
        let mut fields: Vec<(&str, &str)> =
            form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        if let Server::Premium { username, key } = &self.server {
            fields.push(("username", username));
            fields.push(("apiKey", key));
        }
        let mut response = self
            .agent
            .post(&url)
            .send_form(fields)
            .map_err(|e| format!("{}: {e}", self.server.host()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("{}: {e}", self.server.host()))?;
        match status {
            200 => Ok(body),
            429 => Err(format!("{}: too many requests (429)", self.server.host())),
            _ => Err(format!(
                "{} answered {status}: {}",
                self.server.host(),
                body.chars().take(200).collect::<String>()
            )),
        }
    }
}

impl LanguageTool {
    /// Whether the server answers, and what it is: `LanguageTool 6.8`.
    pub fn probe(&self) -> Result<String, String> {
        let form = [
            ("text".to_owned(), "Hello.".to_owned()),
            ("language".to_owned(), "en-US".to_owned()),
        ];
        let answer = self.post(&form)?;
        let answer: serde_json::Value = serde_json::from_str(&answer)
            .map_err(|e| format!("{} is not a LanguageTool server: {e}", self.server.host()))?;
        let software = &answer["software"];
        Ok(format!(
            "{} {}",
            software["name"].as_str().unwrap_or("LanguageTool"),
            software["version"].as_str().unwrap_or("?")
        ))
    }
}

impl Checker for LanguageTool {
    fn check(&self, paragraph: &Paragraph<'_>) -> Vec<Finding> {
        self.check_all(std::slice::from_ref(paragraph))
            .pop()
            .unwrap_or_default()
    }

    fn check_all(&self, paragraphs: &[Paragraph<'_>]) -> Vec<Vec<Finding>> {
        let mut out = vec![Vec::new(); paragraphs.len()];
        let mut error = None;
        for batch in batches(paragraphs, &self.options) {
            let chars = batch
                .paragraphs
                .iter()
                .map(|&i| paragraphs[i].text.chars().count())
                .sum();
            self.throttle(chars);
            match self
                .post(&batch.form)
                .and_then(|answer| batch.findings(&answer))
            {
                Ok(found) => {
                    for (at, finding) in found {
                        out[at].push(finding);
                    }
                }
                Err(e) => error = Some(e),
            }
        }
        *self.error.lock().unwrap() = error;
        out
    }
}
