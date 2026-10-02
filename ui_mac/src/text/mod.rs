// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The portable half of the page (M6): its numbers, its faces, its state and what a frame of it
//! draws — every decision `page_view.rs` puts on the screen, tested on any host.

pub mod blink;
pub mod face;
pub mod format;
pub mod geom;
pub mod input;
pub mod paint;
pub mod picture;
pub mod search;
pub mod state;
