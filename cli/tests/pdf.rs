// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's pages from the outside (`doc/pdf-export.md`): the page a document
//! states, how it breaks into pages, and the PDF and preview it exports to.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn grind(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grind"))
        .args(args)
        .env_remove("GRIND_LOCALE")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HOME")
        .output()
        .expect("the binary runs")
}

fn ok(args: &[&str]) -> String {
    let output = grind(args);
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8")
}

fn writer(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../text/tests/data")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("grind-pdf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("sandbox");
        Sandbox(dir)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn info_names_the_page_a_text_document_states() {
    let info = ok(&["info", &writer("numbered-list.fodt")]);
    assert!(
        info.contains("page\tA4\t210.01 × 297 mm\tmargins 20 20 20 20 mm"),
        "{info}"
    );

    let json = ok(&["--format", "json", "info", &writer("picture.fodt")]);
    assert!(json.contains("\"page\":{"), "{json}");
    assert!(json.contains("\"stated\":true"), "{json}");
    assert!(json.contains("\"width_mm\":215.9"), "{json}");
}

#[test]
fn a_document_stating_no_page_reports_the_a4_it_will_print_on() {
    let dir = Sandbox::new("no-page");
    let file = dir.path("new.fodt");
    ok(&["text", "new", &file]);
    let info = ok(&["info", &file]);
    assert!(
        info.contains("page\tA4\t210 × 297 mm\tmargins 20 20 20 20 mm\t(default)"),
        "{info}"
    );
}
