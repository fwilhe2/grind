// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The About dialog: which build this is, and every third-party component with its licence
//! (`doc/third-party.md`) — the list every window's About shows, from `grind_core::third_party`.
//!
//! A native `<dialog>` rather than a sixth pane: it is read and dismissed, it has nothing to do
//! with the document under it, and the browser already gives it Esc, focus and a backdrop.
//! Each component is a `<details>`, so a list of hundreds is one line each until opened.

use grind_core::third_party;

/// The dialog's content.
pub fn html() -> String {
    let mut out = String::new();
    out.push_str("<h2>Grind</h2>");
    out.push_str(&format!(
        "<pre class=\"about-build\">{}</pre>",
        escape(&grind_core::build_info::describe(
            "grind-web",
            env!("CARGO_PKG_VERSION")
        ))
    ));
    out.push_str(
        "<p>Free software under the GNU Affero General Public License, version 3 or later. \
         <a href=\"https://github.com/fwilhe2/grind\" target=\"_blank\" rel=\"noopener\">\
         Source</a></p>",
    );
    out.push_str(&format!(
        "<h3>Third-party components ({})</h3><div class=\"about-list\">",
        third_party::components().len()
    ));
    for component in third_party::components() {
        out.push_str(&format!(
            "<details><summary>{} <span class=\"about-licence\">{}</span></summary>",
            escape(&component.title()),
            escape(component.licence)
        ));
        if !component.copyright.is_empty() {
            out.push_str(&format!("<p>{}</p>", escape(component.copyright)));
        }
        out.push_str(&format!(
            "<pre>{}</pre></details>",
            escape(&component.text())
        ));
    }
    out.push_str("</div>");
    out.push_str(
        "<form method=\"dialog\"><button class=\"about-close\" type=\"submit\">Close</button></form>",
    );
    out
}

pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_component_is_a_row_and_its_text_is_escaped() {
        let html = html();
        assert_eq!(
            html.matches("<details>").count(),
            third_party::components().len()
        );
        assert!(html.contains("Liberation Fonts 2.1.5"));
        assert!(html.contains("grind-web v"), "the build");
        // An author's address is text here, not a tag.
        assert!(!html.contains("<dtolnay@gmail.com>"));
        assert!(html.contains("&lt;dtolnay@gmail.com&gt;"));
    }
}
