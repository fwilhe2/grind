// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// A UI-free test of the browser boundary.
//
// It drives the real wasm module against the real `index.html`, so a failure here
// is a wasm, glue or wiring problem, and a page that misbehaves after this passes
// is CSS. The interesting one is `opening a document works with no filesystem`:
// that is rule 5 (doc/plan.md) checked end to end — bytes in through the File API,
// `App::open_bytes`, and a value out of the grid.
//
// jsdom has no layout engine — every rectangle it reports is zero — so the shell
// falls back to its metric guards and the viewport is one cell. That is enough to
// exercise everything except how things look. It has no canvas either, which is
// what the text pane measures with; that falls back the same way, so line breaking
// here is plausible rather than accurate, and nothing below asserts a width.
//
// Run it through ui_web/smoke.sh, which builds the pieces it needs.

const fs = require("fs");
const path = require("path");
const { JSDOM, VirtualConsole } = require("jsdom");

const here = __dirname;
const html = fs
  .readFileSync(path.join(here, "index.html"), "utf8")
  // The page loads the ES module build; this harness requires the node one below.
  .replace(/<script type="module">[\s\S]*?<\/script>/, "");

// jsdom shouts "Not implemented" through its own console for every browser API it
// does not have — `canvas.getContext` is the one this shell asks for. The shell
// already treats a missing canvas as "measure it yourself" and carries on, so the
// stack trace is noise in front of the checks. Anything else still comes through.
const virtualConsole = new VirtualConsole();
virtualConsole.sendTo(console, { omitJSDOMErrors: true });
virtualConsole.on("jsdomError", (error) => {
  if (!/not implemented/i.test(error.message)) console.error(error);
});

const dom = new JSDOM(html, {
  pretendToBeVisual: true,
  url: "http://localhost/",
  virtualConsole,
});

// The generated glue type-checks values with `instanceof Window`, `instanceof
// HTMLButtonElement` and so on, so every DOM constructor has to be a real global.
global.window = dom.window;
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (key in global) continue;
  try {
    global[key] = dom.window[key];
  } catch {
    // Some window properties are getters that throw outside a browser; skip them.
  }
}
global.document = dom.window.document;

// Downloading is the one thing jsdom will not do: no object URLs, and an anchor
// click that navigates nowhere. Stubbed rather than skipped, so the save path
// still runs — what it produced is checked below.
// Patched on the global rather than on jsdom's window: node has its own `URL` and
// `Blob`, and the loop above left those alone — so the glue reaches node's.
const downloads = [];
globalThis.URL.createObjectURL = (blob) => {
  downloads.push(blob);
  return "blob:smoke";
};
globalThis.URL.revokeObjectURL = () => {};
dom.window.HTMLAnchorElement.prototype.click = function () {
  downloads.at(-1).name = this.download;
};

// Instantiating the module runs `start()`, which wires the page up.
require(path.join(here, ".smoke/grind_web.js"));

const byId = (id) => document.getElementById(id);
const frame = () => new Promise((resolve) => dom.window.requestAnimationFrame(resolve));

// jsdom reports every rectangle as zero, so the viewport is one cell and the only
// cell reliably on screen is the active one — which is the one every check below
// is about anyway.
const shown = () => document.querySelector("td.active").textContent;

const press = (key, modifiers = {}) =>
  byId("surface").dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers })
  );

// The document pane has its own keyboard: it holds the caret, and every key it
// claims is answered by grind-text rather than by the page.
const typeInDoc = (text) => {
  for (const key of text) pressInDoc(key);
};

const pressInDoc = (key, modifiers = {}) =>
  byId("page").dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers })
  );

// What the document pane is showing, block by block — the same rule as `shown()`:
// read the page, never the core, because the page is what a reader sees.
const blocks = () =>
  [...document.querySelectorAll("#flow .block")].map((block) =>
    [...block.querySelectorAll(".line")].map((line) => line.textContent).join("")
  );

// Typing into the formula bar the way a user does: the key opens the edit and
// seeds it, and the rest goes in as text, each keystroke firing `input`.
const type = (text) => {
  press(text[0]);
  byId("formula").value = text;
  byId("formula").dispatchEvent(new dom.window.Event("input", { bubbles: true }));
};

// The command palette: Ctrl+K anywhere, then type, then Enter. Dispatched on the
// document because that is where the shell listens for it — in the capture phase,
// so no pane can swallow it.
const palette = (key, modifiers = {}) =>
  document.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers })
  );

const paletteType = (text) => {
  byId("palette-input").value = text;
  byId("palette-input").dispatchEvent(new dom.window.Event("input", { bubbles: true }));
};

const paletteRows = () =>
  [...document.querySelectorAll("#palette-list li")].map((row) => row.textContent);

const paletteEnter = () =>
  byId("palette-input").dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })
  );

// Running one command, the way a reader does: open, type enough to pick it out, Enter.
const command = async (query) => {
  palette("k", { ctrlKey: true });
  paletteType(query);
  paletteEnter();
  await frame();
};

const press_button = async (id) => {
  byId(id).dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await frame();
};

// A clipboard event, with the data object the shell reads. jsdom has no
// ClipboardEvent, so this is an ordinary Event carrying the one property the
// handler asks for — which is all wasm-bindgen's cast looks at.
const clipboardEvent = (kind, text) => {
  const data = {
    store: text ?? "",
    getData() {
      return this.store;
    },
    setData(_type, value) {
      this.store = value;
    },
  };
  const event = new dom.window.Event(kind, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: data });
  document.dispatchEvent(event);
  return data;
};

const enter = async (text) => {
  type(text);
  press("Enter");
  await frame();
};

let failures = 0;
const check = (label, actual, expected) => {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  if (!ok) failures += 1;
  console.log(
    `${ok ? "ok  " : "FAIL"} ${label}` +
      (ok
        ? ""
        : `\n       expected ${JSON.stringify(expected)}\n       got      ${JSON.stringify(actual)}`)
  );
};

// A minimal flat ODF document, so the open path is fed a real file rather than a
// mock. Flat rather than a package because a zip cannot be written in a line here —
// the binary path is what the browser's own file picker exercises.
const FODS = `<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0"
    office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
  <office:body><office:spreadsheet>
    <table:table table:name="Opened">
      <table:table-row><table:table-cell office:value-type="string"><text:p>from a file</text:p></table:table-cell></table:table-row>
    </table:table>
  </office:spreadsheet></office:body>
</office:document>`;

// A minimal flat text document, so the text pane is fed a real file too. Its
// kind is read from the bytes, which is why it can be told apart from the
// spreadsheet above without looking at either name.
const FODT = `<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
  <office:body><office:text>
    <text:h text:outline-level="1">Title</text:h>
    <text:p>One paragraph.</text:p>
  </office:text></office:body>
</office:document>`;

// A spreadsheet with a chart in it, so the SVG path is fed a real one.
const CHART = `<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
    xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0"
    xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
    office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
  <office:body><office:spreadsheet>
    <table:table table:name="Sheet1">
      <table:shapes>
        <draw:frame svg:x="1cm" svg:y="1cm" svg:width="8cm" svg:height="5cm">
          <draw:object>
            <office:document office:mimetype="application/vnd.oasis.opendocument.chart">
              <office:body><office:chart>
                <chart:chart svg:width="8cm" svg:height="5cm" chart:class="chart:bar">
                  <chart:plot-area>
                    <chart:axis chart:dimension="x">
                      <chart:categories table:cell-range-address="Sheet1.A1:Sheet1.A2"/>
                    </chart:axis>
                    <chart:axis chart:dimension="y"/>
                    <chart:series chart:class="chart:bar"
                                  chart:values-cell-range-address="Sheet1.B1:Sheet1.B2">
                      <chart:data-point chart:repeated="2"/>
                    </chart:series>
                  </chart:plot-area>
                </chart:chart>
              </office:chart></office:body>
            </office:document>
          </draw:object>
        </draw:frame>
      </table:shapes>
      <table:table-row>
        <table:table-cell office:value-type="string"><text:p>one</text:p></table:table-cell>
        <table:table-cell office:value-type="float" office:value="5"/>
      </table:table-row>
      <table:table-row>
        <table:table-cell office:value-type="string"><text:p>two</text:p></table:table-cell>
        <table:table-cell office:value-type="float" office:value="9"/>
      </table:table-row>
    </table:table>
  </office:spreadsheet></office:body></office:document>`;

// A text document with formatting in it — a Title, and a bold run inside a paragraph.
const RICH = `<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
    xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
    office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
  <office:automatic-styles>
    <style:style style:name="T1" style:family="text">
      <style:text-properties fo:font-weight="bold"/>
    </style:style>
  </office:automatic-styles>
  <office:body><office:text>
    <text:p text:style-name="Title">The Title</text:p>
    <text:p>Plain and <text:span text:style-name="T1">bold</text:span>.</text:p>
    <text:h text:outline-level="1">A Heading</text:h>
  </office:text></office:body></office:document>`;

(async () => {
  await frame();
  check("the grid is drawn from the core", document.querySelectorAll("td.cell").length > 0, true);
  check("the address follows the selection", byId("address").value, "A1");

  await enter("12");
  check("Enter moves down", byId("address").value, "A2");
  await enter("30");

  press("ArrowUp");
  press("ArrowUp");
  await frame();
  check("the arrows move", byId("address").value, "A1");
  check("typing reaches the core", shown(), "12");

  press("ArrowRight");
  await frame();
  // The formula bar speaks A1; the document stores ODF's own syntax. That
  // conversion is the core's, and this is the check that the shell uses it.
  type("=SUM(A1:A2)");
  press("Enter");
  press("ArrowUp");
  await frame();
  check("a formula is evaluated", shown(), "42");
  check("and shown back in A1 form", byId("formula").value, "=SUM(A1:A2)");

  type("=SUM(");
  press("Enter");
  await frame();
  check("a bad formula keeps the edit open", byId("message").textContent !== "", true);
  press("Escape");
  await frame();
  check("Escape leaves the cell alone", shown(), "42");

  press("z", { ctrlKey: true });
  await frame();
  check("Ctrl+Z undoes", shown(), "");
  press("z", { ctrlKey: true, shiftKey: true });
  await frame();
  check("Ctrl+Shift+Z redoes", shown(), "42");
  press("z", { metaKey: true });
  await frame();
  check("⌘Z is the same shortcut", shown(), "");
  check("the buttons follow the history", byId("redo").disabled, false);

  press("y", { ctrlKey: true });
  await frame();

  // A key the browser owns must not be typed into a cell.
  press("t", { ctrlKey: true });
  await frame();
  check("browser shortcuts are not typed", byId("address").value, "B1");
  check("nothing was entered", shown(), "42");

  press("Delete");
  await frame();
  check("Delete clears the selection", shown(), "");

  // Saving: the bytes the core produced, not the download the browser refuses to
  // perform. Flat by default (doc/flat-first.md), so what comes out is XML rather
  // than a zip — the one place in this file that would notice the decision changing.
  byId("save").dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await frame();
  check("saving names the download", downloads.at(-1).name, "untitled.fods");
  const saved = new TextDecoder().decode(await downloads.at(-1).arrayBuffer());
  check("and produces flat ODF, not a zip", saved.slice(0, 5), "<?xml");
  check("which says what kind of document it is",
        saved.includes("opendocument.spreadsheet"), true);

  // Rule 5, end to end: a document arrives as bytes and there is no path anywhere.
  // Node's `File`, not jsdom's: jsdom's has no `arrayBuffer()`, which is the one
  // method this path needs and the one every browser has.
  const file = new File([FODS], "opened.fods", { type: "text/xml" });
  Object.defineProperty(byId("file-input"), "files", { value: [file], configurable: true });
  byId("file-input").dispatchEvent(new dom.window.Event("change", { bubbles: true }));
  await frame();
  await frame();
  check("opening a document works with no filesystem", shown(), "from a file");
  check("and the sheet's own name is shown", byId("summary").textContent.startsWith("Opened"), true);
  check("the name travels with it", byId("name").textContent, "opened.fods");

  // --- the word processor -------------------------------------------------
  //
  // R10: every document type reaches every shell. One bundle, and which pane is
  // showing is decided by `grind_core::kind` from the bytes — never the name.
  const openFile = async (name, content) => {
    const file = new File([content], name, { type: "text/xml" });
    Object.defineProperty(byId("file-input"), "files", { value: [file], configurable: true });
    byId("file-input").dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    await frame();
    await frame();
  };

  await openFile("notes.fodt", FODT);
  check("a text document opens the document pane", byId("page").hidden, false);
  check("and puts the grid away", byId("surface").hidden, true);
  check("the formula bar goes with the grid", byId("formula-bar").hidden, true);
  check("the document is drawn from the core", blocks(), ["Title", "One paragraph."]);

  // Typing: every key is `App::insert_text`, and the caret is an element in the line.
  pressInDoc("ArrowDown");
  pressInDoc("End");
  typeInDoc(" More.");
  await frame();
  check("typing reaches the core", blocks(), ["Title", "One paragraph. More."]);
  check("the caret is in the document", document.querySelectorAll("#caret").length, 1);

  // Enter splits a block and Backspace at the front joins it back — the two edits
  // that make a flat sequence of blocks behave like one flow of text.
  pressInDoc("Enter");
  await frame();
  check("Enter splits a block", blocks().length, 3);
  pressInDoc("Backspace");
  await frame();
  check("Backspace at the front joins it back", blocks(), ["Title", "One paragraph. More."]);

  // The chrome's shortcuts work in both panes and reach the core that is showing.
  pressInDoc("z", { ctrlKey: true });
  await frame();
  check("Ctrl+Z undoes in whichever document is open", blocks().length, 3);

  byId("save").dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await frame();
  check("saving names the download after the document", downloads.at(-1).name, "notes.fodt");
  const written = await downloads.at(-1).text();
  check("and writes a flat text document", written.includes("office:text"), true);

  // Back to a spreadsheet: the same bundle, the same buttons, the other pane.
  await openFile("again.fods", FODS);
  check("a spreadsheet brings the grid back", byId("surface").hidden, false);
  check("and the document pane goes away", byId("page").hidden, true);
  check("with the sheet drawn again", shown(), "from a file");

  // --- the command palette ------------------------------------------------
  //
  // This shell's menu bar. Every verb either pane has is in it, it is the only way
  // to reach some of them, and it doubles as the go-to box.
  palette("k", { ctrlKey: true });
  check("Ctrl+K opens the palette", byId("palette").hidden, false);
  check("and it starts with the common commands", paletteRows().length > 0, true);
  paletteType("bol");
  check("typing filters it", paletteRows()[0].startsWith("Bold"), true);
  palette("Escape");
  check("Escape closes it", byId("palette").hidden, true);

  // Going somewhere: an address typed into the palette is a destination, not a verb.
  await openFile("again2.fods", FODS);
  palette("k", { ctrlKey: true });
  paletteType("C7");
  check("an address is offered as somewhere to go", paletteRows()[0].startsWith("Go to C7"), true);
  paletteEnter();
  await frame();
  check("and going there moves the selection", byId("address").value, "C7");

  // The address box is the other half of the same idea, for people who reach for it.
  byId("address").value = "B2";
  byId("address").dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })
  );
  await frame();
  check("the address box goes where it is told", byId("address").value, "B2");

  // --- formatting -----------------------------------------------------------
  //
  // The toolbar and the palette run the same command id, so this checks both.
  await enter("7");
  press("ArrowUp");
  await frame();
  await press_button("s-bold");
  check("the toolbar makes a cell bold", document.querySelector("td.active").getAttribute("style").includes("font-weight:bold"), true);
  check("and the toggle shows it", byId("s-bold").getAttribute("aria-pressed"), "true");
  await press_button("s-bold");
  check("pressing it again turns it off", byId("s-bold").getAttribute("aria-pressed"), "false");

  await command("Number: per cent");
  await frame();
  check("a number format changes what is shown", shown(), "700%");
  check("and the picker reports it", byId("s-format").value, "format.percent");
  await command("Number: general");
  await frame();
  check("General puts it back", shown(), "7");

  // --- the clipboard --------------------------------------------------------
  //
  // The browser's own events, which is the path Ctrl+C actually takes.
  const copied = clipboardEvent("copy");
  check("copying writes the cell's own input text", copied.store, "7");
  press("ArrowDown");
  await frame();
  clipboardEvent("paste", "11\t12\n13\t14");
  await frame();
  check("pasting enters a rectangle from where it started", shown(), "11");
  press("ArrowRight");
  await frame();
  check("across", shown(), "12");
  press("ArrowDown");
  await frame();
  check("and down", shown(), "14");

  // --- charts ---------------------------------------------------------------
  //
  // Drawn as SVG from the same `ChartData` and the same scale the GTK shell uses.
  await openFile("chart.fods", CHART);
  check("a chart in the file is drawn", document.querySelectorAll("#charts .chart svg").length, 1);
  check("with a mark per bar", document.querySelectorAll("#charts .chart rect").length, 2);

  // --- the document pane, again ---------------------------------------------
  await openFile("rich.fodt", RICH);
  check("a bold run is drawn bold", document.querySelectorAll("#flow .run.b").length > 0, true);
  check("and a Title has its own class", document.querySelectorAll("#flow .block.title").length, 1);

  // Selection: Shift+arrow extends it, and what is selected can be formatted.
  pressInDoc("ArrowDown");
  pressInDoc("Home");
  for (let i = 0; i < 5; i += 1) pressInDoc("ArrowRight", { shiftKey: true });
  await frame();
  check("Shift+arrow selects", document.querySelectorAll("#flow .sel").length > 0, true);
  const selected = clipboardEvent("copy");
  check("and the selection is what gets copied", selected.store, "Plain");

  pressInDoc("u", { ctrlKey: true });
  await frame();
  check("Ctrl+U underlines the selection", document.querySelectorAll("#flow .run.u").length > 0, true);

  // Markdown-shaped typing, the same reading the terminal and the CLI do
  // (grind_text::markdown, one core call).
  pressInDoc("End");
  typeInDoc(" run `ls` now");
  await frame();
  check("typing markdown formats the span", blocks().some((b) => b.includes("run ls now")), true);
  check("and takes the markers out", blocks().every((b) => !b.includes("`")), true);
  check("the code run is drawn monospace",
        [...document.querySelectorAll("#flow .run")].some(
          (r) => (r.getAttribute("style") || "").includes("font-family:monospace")),
        true);

  pressInDoc("2", { ctrlKey: true });
  await frame();
  check("Ctrl+2 makes the block a heading", document.querySelectorAll("#flow .block.h2").length, 1);
  check("and the picker reports it", byId("t-block").value, "block.h2");

  // The palette's own outline: a heading is somewhere to go.
  palette("k", { ctrlKey: true });
  check("the palette offers the outline", paletteRows().some((row) => row.includes("A Heading")), true);
  palette("Escape");

  // The code view (doc/dsl.md §6, D9): the document as its projection, the caret's own line
  // drawn as current, and clicking a line putting the caret in the block it projects.
  await command("source");
  const codeLines = () => [...document.querySelectorAll("#code .code-line")];
  check("the code view shows the projection", codeLines().length > 0, true);
  check("with the writer's own colours",
        document.querySelectorAll("#code .code-node").length > 0, true);
  check("and the caret's line is current",
        document.querySelectorAll("#code .code-line-current").length, 1);
  check("the document pane is off screen while it shows", byId("page").hidden, true);

  const firstLine = codeLines().find((line) => line.textContent.includes("h 1"))
    || codeLines().find((line) => line.querySelector(".code-node"));
  firstLine.querySelector(".code-node").dispatchEvent(
    new dom.window.MouseEvent("click", { bubbles: true })
  );
  await frame();
  check("clicking a line makes it the current one",
        document.querySelector("#code .code-line-current"), firstLine);

  await command("source");
  check("running it again puts the document back", byId("code").hidden, true);
  check("and the pane is showing", byId("page").hidden, false);

  // The problems pane (doc/dsl.md §4.3, D6): `grind lint`'s findings, and a row that goes
  // where it points. This document is a text one, so the rules that can fire are the word
  // processor's — what is checked here is the pane, not which rule spoke.
  await command("Check the document");
  check("the problems pane opens", byId("problems").hidden, false);
  check("the document pane is off screen while it shows", byId("page").hidden, true);
  check("and it says what it found", byId("problems").textContent.length > 0, true);

  const problem = document.querySelector("#problems .problem");
  if (problem) {
    problem.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
    await frame();
    check("clicking a finding puts the document back", byId("problems").hidden, true);
  } else {
    await command("Check the document");
    check("running it again puts the document back", byId("problems").hidden, true);
  }
  check("and the pane is showing", byId("page").hidden, false);

  // A repaint that changes nothing must still be safe — a resize borrows the same
  // message it writes back.
  const message = byId("message").textContent;
  dom.window.dispatchEvent(new dom.window.Event("resize"));
  await frame();
  check("a resize repaints and keeps the message", byId("message").textContent, message);

  console.log(failures === 0 ? "\nall checks passed" : `\n${failures} check(s) failed`);
  process.exit(failures === 0 ? 0 : 1);
})();
