// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The documents opened or saved most recently, **kept in this browser**: a page cannot open a
//! file again by its path, so each is a copy of its bytes in `localStorage`, newest first. A
//! document too large to keep is not listed; nothing here is ever an error on screen.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::command::Entry;

/// The names, newest first, one a line.
const INDEX: &str = "grind.recent";
/// Each document's bytes, base64, under this prefix and its place in the index.
const BYTES: &str = "grind.recent.";
/// How many are kept. With [`LARGEST`], four stay inside the 5 MB a browser gives a page.
pub const KEPT: usize = 4;
const LARGEST: usize = 768 * 1024;

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The names kept, newest first.
pub fn names() -> Vec<String> {
    storage()
        .and_then(|s| s.get_item(INDEX).ok().flatten())
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The bytes kept for the `n`th name.
pub fn bytes(n: usize) -> Option<(String, Vec<u8>)> {
    let name = names().into_iter().nth(n)?;
    let text = storage()?.get_item(&format!("{BYTES}{n}")).ok()??;
    Some((name, STANDARD.decode(text).ok()?))
}

/// Put a copy of `bytes` at the head of the list, under `name`.
pub fn note(name: &str, bytes: &[u8]) {
    let Some(storage) = storage() else { return };
    let mut kept: Vec<(String, String)> = names()
        .into_iter()
        .enumerate()
        .filter_map(|(n, name)| Some((name, storage.get_item(&format!("{BYTES}{n}")).ok()??)))
        .collect();
    kept.retain(|(kept, _)| kept != name);
    if bytes.len() <= LARGEST && !name.contains('\n') {
        kept.insert(0, (name.to_owned(), STANDARD.encode(bytes)));
    }
    kept.truncate(KEPT);
    forget();
    let mut index = String::new();
    for (n, (name, text)) in kept.iter().enumerate() {
        // Past the browser's quota the rest are dropped rather than half-kept.
        if storage.set_item(&format!("{BYTES}{n}"), text).is_err() {
            break;
        }
        index.push_str(name);
        index.push('\n');
    }
    let _ = storage.set_item(INDEX, &index);
}

/// Drop every copy.
pub fn forget() {
    let Some(storage) = storage() else { return };
    let _ = storage.remove_item(INDEX);
    for n in 0..KEPT {
        let _ = storage.remove_item(&format!("{BYTES}{n}"));
    }
}

/// The palette's rows for `names`: on the welcome pane all of them, elsewhere those the query
/// finds, and a row to forget them all.
pub fn entries(query: &str, names: &[String], welcome: bool) -> Vec<Entry> {
    let query = query.trim().to_lowercase();
    if names.is_empty() || (query.is_empty() && !welcome) {
        return Vec::new();
    }
    let rows = names
        .iter()
        .enumerate()
        .map(|(n, name)| (format!("doc.recent:{n}"), format!("Open {name}")))
        .chain([("doc.forget-recent".to_owned(), FORGET.to_owned())]);
    rows.filter(|(_, title)| title.to_lowercase().contains(&query))
        .map(|(id, title)| Entry::target(id, title, "Recent"))
        .collect()
}

const FORGET: &str = "Forget the recent documents";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_welcome_pane_lists_every_one_and_a_query_finds_one() {
        let names = ["budget.fods".to_owned(), "letter.fodt".to_owned()];
        let ids = |entries: Vec<Entry>| entries.into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(
            ids(entries("", &names, true)),
            ["doc.recent:0", "doc.recent:1", "doc.forget-recent"]
        );
        assert!(entries("", &names, false).is_empty());
        assert_eq!(ids(entries("lett", &names, false)), ["doc.recent:1"]);
        assert_eq!(ids(entries("forget", &names, false)), ["doc.forget-recent"]);
        assert!(entries("", &[], true).is_empty());
    }
}
