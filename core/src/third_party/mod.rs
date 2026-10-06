// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every third-party component the suite's programs are built from, and the notices its licence
//! asks to be passed on — what each shell's About shows and `grind licences` prints
//! (`doc/third-party.md`).
//!
//! The list is **generated, never written by hand**: `core/tests/third_party.rs` walks
//! `cargo metadata` from every binary in the workspace, reads each crate's own licence files
//! out of the crate, adds what Cargo does not know about (the bundled fonts, the Rust standard
//! library), and fails when `data` differs from what it would write — rewriting it as it fails,
//! so the next `git diff` is the review. Here in the core because every shell already depends on
//! it, and because one list read by six shells cannot be six lists.

mod data;

/// What sort of thing a component is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A Rust crate from the dependency graph.
    Crate,
    /// A font compiled into a binary or shipped beside one.
    Font,
    /// Text taken from a specification: the formula functions' summaries.
    Text,
    /// Part of the toolchain that ends up in every binary: the Rust standard library.
    Runtime,
    /// A spelling dictionary compiled into a binary (`doc/spelling.md`).
    Dictionary,
}

/// One third-party component.
#[derive(Clone, Copy, Debug)]
pub struct Component {
    pub name: &'static str,
    /// Empty where there is no one version to name (the standard library is whichever
    /// toolchain built the binary).
    pub version: &'static str,
    /// The licence as its authors state it — an SPDX expression for a crate.
    pub licence: &'static str,
    /// The first copyright line its notices carry, empty where they carry none.
    pub copyright: &'static str,
    /// Where it comes from: its repository, or its home page.
    pub source: &'static str,
    pub kind: Kind,
    notices: &'static [usize],
}

impl Component {
    /// Its licence files, verbatim.
    pub fn notices(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.notices.iter().map(|&at| data::NOTICES[at])
    }

    /// Every licence file it carries as one text, then where it comes from — what a shell
    /// shows for one component.
    pub fn text(&self) -> String {
        let mut text = self.notices().collect::<Vec<_>>().join("\n\n");
        text.push_str("\n\n");
        text.push_str(self.source);
        text
    }

    /// `name version`, or just the name.
    pub fn title(&self) -> String {
        match self.version {
            "" => self.name.to_owned(),
            version => format!("{} {version}", self.name),
        }
    }
}

/// Every component, sorted by name.
pub fn components() -> &'static [Component] {
    data::COMPONENTS
}

/// The whole list as one plain text — what `grind licences` prints and a shell with nowhere
/// better to put it shows in a scrolled pane. An index first, then each distinct notice once,
/// headed by every component it belongs to.
pub fn notices() -> String {
    let mut out = String::from(
        "Grind is free software under the GNU Affero General Public License, version 3 or later.\n\
         It is built from the third-party components below, each under its own licence.\n\n",
    );
    for component in components() {
        out.push_str(&format!(
            "  {}  ({})\n",
            component.title(),
            component.licence
        ));
    }
    for (at, text) in data::NOTICES.iter().enumerate() {
        let users: Vec<String> = components()
            .iter()
            .filter(|c| c.notices.contains(&at))
            .map(Component::title)
            .collect();
        out.push_str("\n\n");
        out.push_str(&"=".repeat(72));
        out.push('\n');
        out.push_str(&users.join(", "));
        out.push('\n');
        out.push_str(&"=".repeat(72));
        out.push_str("\n\n");
        out.push_str(text.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_component_has_a_notice_and_every_notice_a_component() {
        let mut used = vec![false; data::NOTICES.len()];
        for component in components() {
            assert!(
                !component.notices.is_empty(),
                "{} has no notice",
                component.name
            );
            for &at in component.notices {
                used[at] = true;
            }
        }
        assert!(used.iter().all(|&u| u), "a notice nobody uses");
    }

    #[test]
    fn the_fonts_and_the_standard_library_are_on_the_list() {
        let kinds: Vec<Kind> = components().iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&Kind::Font));
        assert!(kinds.contains(&Kind::Runtime));
        assert!(kinds.contains(&Kind::Dictionary));
        assert!(kinds.contains(&Kind::Crate));
    }

    #[test]
    fn the_text_names_every_component() {
        let text = notices();
        for component in components() {
            assert!(text.contains(&component.title()), "{}", component.name);
        }
    }
}
