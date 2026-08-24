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

(async () => {
  await frame();
  check("the grid is drawn from the core", document.querySelectorAll("td.cell").length > 0, true);
  check("the address follows the selection", byId("address").textContent, "A1");

  await enter("12");
  check("Enter moves down", byId("address").textContent, "A2");
  await enter("30");

  press("ArrowUp");
  press("ArrowUp");
  await frame();
  check("the arrows move", byId("address").textContent, "A1");
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
  check("browser shortcuts are not typed", byId("address").textContent, "B1");
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

  // A repaint that changes nothing must still be safe — a resize borrows the same
  // message it writes back.
  const message = byId("message").textContent;
  dom.window.dispatchEvent(new dom.window.Event("resize"));
  await frame();
  check("a resize repaints and keeps the message", byId("message").textContent, message);

  console.log(failures === 0 ? "\nall checks passed" : `\n${failures} check(s) failed`);
  process.exit(failures === 0 ? 0 : 1);
})();
