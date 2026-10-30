// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which LanguageTool server is asked, and what it allows (`doc/grammar.md` decision 9).

/// A LanguageTool server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Server {
    /// One the person runs, at this URL: usually a container on this computer.
    SelfHosted(String),
    /// LanguageTool's free public API.
    Public,
    /// LanguageTool Premium, with the account's username and API key.
    Premium { username: String, key: String },
}

impl Server {
    /// The address `/v2/check` is under.
    pub fn base(&self) -> &str {
        match self {
            Server::SelfHosted(url) => url.trim_end_matches('/'),
            Server::Public => "https://api.languagetool.org",
            Server::Premium { .. } => "https://api.languagetoolplus.com",
        }
    }

    /// The host the text goes to.
    pub fn host(&self) -> &str {
        let rest = self
            .base()
            .split_once("://")
            .map_or(self.base(), |(_, r)| r);
        let host_port = rest.split('/').next().unwrap_or(rest);
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    }

    /// Whether the text stays on this computer.
    pub fn local(&self) -> bool {
        matches!(self.host(), "localhost" | "127.0.0.1" | "[::1]" | "::1")
    }

    /// Where the text goes, in words — what a shell shows when the server is chosen and while it
    /// is on.
    pub fn describe(&self) -> String {
        match self.local() {
            true => "checked on this computer".to_owned(),
            false => format!("your text is sent to {}", self.host()),
        }
    }

    /// The most text one request may carry, in characters: what the public and Premium servers
    /// allow, and a size that keeps one's own server answering quickly.
    pub fn max_chars(&self) -> usize {
        match self {
            Server::SelfHosted(_) => 100_000,
            Server::Public => 20_000,
            Server::Premium { .. } => 60_000,
        }
    }

    /// Requests and characters a minute it allows, where it says.
    pub fn per_minute(&self) -> Option<(usize, usize)> {
        match self {
            Server::SelfHosted(_) => None,
            Server::Public => Some((20, 75_000)),
            Server::Premium { .. } => Some((80, 300_000)),
        }
    }
}
