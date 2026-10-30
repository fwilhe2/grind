// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The LanguageTool client (`doc/grammar.md` §5): paragraphs into `/v2/check` requests, and the
//! server's matches back into findings. This half does no I/O, so the browser shell sends the
//! same requests through `fetch`.

#[cfg(feature = "native")]
mod client;
pub mod request;
mod server;

#[cfg(feature = "native")]
pub use client::LanguageTool;
pub use request::{Batch, Options, batches};
pub use server::Server;
