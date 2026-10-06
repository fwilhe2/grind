// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! List labels — a list item's number or its style's bullet — derived from its list style and
//! its place in the list, and never stored (`grind_text::numbering`). Every expectation below is
//! what LibreOffice 26.8 drew for the same document (`doc/odt-format.md` §5c, fact 15).

use grind_text::App;

const LISTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.text">
<office:automatic-styles>
<text:list-style style:name="L1">
 <text:list-level-style-number text:level="1" style:num-suffix="." style:num-format="1"/>
 <text:list-level-style-number text:level="2" style:num-suffix=")" style:num-format="a" text:display-levels="2"/>
 <text:list-level-style-number text:level="3" style:num-prefix="(" style:num-suffix=")" style:num-format="i"/>
</text:list-style>
<text:list-style style:name="L2">
 <text:list-level-style-bullet text:level="1" text:bullet-char="–"/>
 <text:list-level-style-number text:level="2" style:num-format="A" style:num-suffix="."/>
</text:list-style>
<text:list-style style:name="L3">
 <text:list-level-style-number text:level="1" style:num-suffix="." style:num-format="I" text:start-value="4"/>
</text:list-style>
</office:automatic-styles>
<office:body><office:text>
<text:list text:style-name="L1">
 <text:list-item><text:p>one</text:p>
  <text:list><text:list-item><text:p>one-a</text:p></text:list-item><text:list-item><text:p>one-b</text:p>
   <text:list><text:list-item><text:p>deep1</text:p></text:list-item><text:list-item><text:p>deep2</text:p></text:list-item></text:list>
  </text:list-item></text:list></text:list-item>
 <text:list-item><text:p>two</text:p><text:p>two, again</text:p><text:list><text:list-item><text:p>two-a</text:p></text:list-item></text:list></text:list-item>
 <text:list-header><text:p>header</text:p></text:list-header>
 <text:list-item><text:p>three</text:p></text:list-item>
</text:list>
<text:p>between</text:p>
<text:list text:style-name="L1"><text:list-item><text:p>restart</text:p></text:list-item></text:list>
<text:list text:style-name="L1" text:continue-numbering="true"><text:list-item><text:p>continued</text:p></text:list-item></text:list>
<text:list text:style-name="L1"><text:list-item text:start-value="7"><text:p>seven</text:p></text:list-item><text:list-item><text:p>eight</text:p></text:list-item></text:list>
<text:list text:style-name="L2"><text:list-item><text:p>dash</text:p><text:list><text:list-item><text:p>capA</text:p></text:list-item></text:list></text:list-item></text:list>
<text:list text:style-name="L3"><text:list-item><text:p>four</text:p></text:list-item><text:list-item><text:p>five</text:p></text:list-item></text:list>
<text:list><text:list-item><text:p>nostyle</text:p></text:list-item></text:list>
<text:list text:style-name="L1"><text:list-item><text:list><text:list-item><text:p>skipped</text:p></text:list-item></text:list></text:list-item></text:list>
</office:text></office:body></office:document>"#;

fn shown(app: &App) -> Vec<(String, Option<String>)> {
    app.get_viewport(0..app.block_count())
        .iter()
        .map(|view| (view.text.clone(), view.mark().map(str::to_owned)))
        .collect()
}

#[test]
fn every_label_is_the_one_libreoffice_draws() {
    let app = App::new();
    app.open_bytes("lists.fodt", LISTS.as_bytes())
        .expect("opens");
    let expected = [
        ("one", Some("1.")),
        ("one-a", Some("1.a)")),
        ("one-b", Some("1.b)")),
        ("deep1", Some("(i)")),
        ("deep2", Some("(ii)")),
        ("two", Some("2.")),
        ("two, again", None),
        ("two-a", Some("2.a)")),
        ("header", None),
        ("three", Some("3.")),
        ("between", None),
        ("restart", Some("1.")),
        ("continued", Some("2.")),
        ("seven", Some("7.")),
        ("eight", Some("8.")),
        ("dash", Some("–")),
        ("capA", Some("A.")),
        ("four", Some("IV.")),
        ("five", Some("V.")),
        // LibreOffice draws nothing for a list naming no style; this build draws its own bullet,
        // which is what every list it writes would otherwise lose.
        ("nostyle", Some("\u{2022}")),
        ("skipped", Some("1.a)")),
    ];
    let got = shown(&app);
    let got: Vec<(&str, Option<&str>)> = got
        .iter()
        .map(|(t, m)| (t.as_str(), m.as_deref()))
        .collect();
    assert_eq!(got, expected);
}

/// An item made by editing has no list of its own in the file; it takes the one of the item
/// before it, so Enter in a numbered list numbers the new item and moves the rest along.
#[test]
fn a_new_item_numbers_itself_and_the_items_after_it_move() {
    let app = App::new();
    app.open_bytes("lists.fodt", LISTS.as_bytes())
        .expect("opens");
    let caret = grind_text::Caret {
        block: 0,
        offset: 3,
    };
    app.split_block(caret).expect("splits");
    let got = shown(&app);
    assert_eq!(got[1].1.as_deref(), Some("2."), "{got:?}");
    assert_eq!(got[6].0, "two");
    assert_eq!(got[6].1.as_deref(), Some("3."), "{got:?}");
}
