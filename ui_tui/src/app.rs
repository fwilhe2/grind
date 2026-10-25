// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What both halves share, which is almost nothing: a grid and a flow have no rendering in common,
//! so there is no shared widget. What they share is the reactive contract — the core pushes, a
//! shell never polls.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::path::PathBuf;

use grind_core::{DocumentKind, Observer};

/// A request from a pane to replace itself — what `:new` and `:open` ask the event loop for, since a
/// pane cannot build its own successor (it may be the other kind of document). The loop swaps
/// the pane and keeps the terminal, which is the in-session "new" and "open" a window has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Switch {
    /// An empty document of this kind.
    New(DocumentKind),
    /// This file, whose kind was read from its bytes by `crate::sniff`.
    Open(PathBuf, DocumentKind),
    /// The pane with no document in it (`crate::welcome`) — what `:welcome` goes back to.
    Welcome,
}

/// Set by the core whenever the document changes; the event loop redraws when it is (rule 3).
///
/// It also **counts** changes, which is how a pane knows whether anything is unsaved: the flag
/// alone cannot say (a pane raises it by hand to repaint), and `App::can_undo` stays true after a
/// save. A pane records [`RedrawFlag::edits`] when it saves; anything since is unsaved.
#[derive(Default)]
pub struct RedrawFlag {
    raised: AtomicBool,
    edits: AtomicU64,
}

impl RedrawFlag {
    pub fn take(&self) -> bool {
        self.raised.swap(false, Ordering::SeqCst)
    }

    pub fn raise(&self) {
        self.raised.store(true, Ordering::SeqCst);
    }

    /// How many times the document has changed, ever. Only its equality with an earlier reading
    /// means anything.
    pub fn edits(&self) -> u64 {
        self.edits.load(Ordering::SeqCst)
    }
}

impl Observer for RedrawFlag {
    fn changed(&self) {
        self.edits.fetch_add(1, Ordering::SeqCst);
        self.raise();
    }
}

/// Hand `text` to the system clipboard with OSC 52, which most terminals honour and the rest
/// ignore. Only when stdout is the terminal, so a test's captured output stays clean.
pub fn copy_to_terminal(text: &str) {
    use std::io::{IsTerminal, Write};
    let mut out = std::io::stdout();
    if out.is_terminal() {
        let _ = write!(out, "{}", osc52(text));
        let _ = out.flush();
    }
}

fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char),
                false => out.push('='),
            }
        }
    }
    out
}

/// A paste as one line, for a line being edited: its line breaks and tabs as spaces, a trailing
/// break dropped.
pub fn one_line(text: &str) -> String {
    unix_lines(text)
        .trim_end_matches('\n')
        .replace(['\n', '\t'], " ")
}

/// A paste's line breaks as `\n`: terminals send a pasted break as `\r`.
pub fn unix_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_base64_inside_osc_52() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("a\tü".as_bytes()), "YQnDvA==");
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn a_paste_has_its_line_breaks_read() {
        assert_eq!(unix_lines("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(one_line("a\tb\r\nc\r"), "a b c");
    }
}
