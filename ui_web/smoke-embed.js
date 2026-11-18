// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The page opened as an embedding page opens it (doc/web-shell.md, "Embedded"): in a frame, with
// `?doc=` naming a document beside it. Run once per outcome, since the module starts once per
// process:
//
//   node smoke-embed.js opens   — the document arrives, and the parent hears it opened
//   node smoke-embed.js fails   — it does not, and the welcome pane says why
//
// The page's own inline script runs (it marks the page as opening before the module is there);
// its module script is replaced by requiring the node build, as in smoke.js.

const fs = require("fs");
const path = require("path");
const { JSDOM, VirtualConsole } = require("jsdom");

const outcome = process.argv[2];
if (outcome !== "opens" && outcome !== "fails") {
  console.error("usage: node smoke-embed.js opens|fails");
  process.exit(2);
}

const here = __dirname;
const html = fs
  .readFileSync(path.join(here, "index.html"), "utf8")
  .replace(/<script type="module">[\s\S]*?<\/script>/, "")
  // jsdom loads no stylesheets; the page's own is what hides the welcome pane while it opens.
  .replace(
    '<link rel="stylesheet" href="style.css" />',
    () => `<style>${fs.readFileSync(path.join(here, "style.css"), "utf8")}</style>`
  );

const virtualConsole = new VirtualConsole();
virtualConsole.sendTo(console, { omitJSDOMErrors: true });
virtualConsole.on("jsdomError", (error) => {
  if (!/not implemented/i.test(error.message)) console.error(error);
});

const name = outcome === "opens" ? "My%20report.fodt" : "missing%20file.fodt";
const dom = new JSDOM(html, {
  pretendToBeVisual: true,
  url: `http://localhost/office/?doc=../dav/files/${name}`,
  runScripts: "dangerously",
  virtualConsole,
});

let failed = 0;
const check = (ok, what) => {
  if (!ok) failed += 1;
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
};

check(dom.window.document.documentElement.hasAttribute("data-opening"),
  "the page is marked as opening before the module is there");
check(dom.window.getComputedStyle(dom.window.document.getElementById("welcome")).display === "none",
  "no welcome pane while it opens");

// The frame's parent: everything posted to it, and to which origin.
const posted = [];
Object.defineProperty(dom.window, "parent", {
  configurable: true,
  get: () => ({ postMessage: (message, origin) => posted.push({ message, origin }) }),
});

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

// The document beside the page, and nothing else: a fetch anywhere but `../dav/files/` fails
// the run, as in smoke.js.
const requested = [];
dom.window.fetch = async (url) => {
  requested.push(String(url));
  if (/^[a-z]+:\/\//i.test(String(url))) throw new Error(`fetch left the page's own files: ${url}`);
  if (String(url) === "../dav/files/My%20report.fodt") {
    const bytes = fs.readFileSync(path.join(here, "../text/tests/data/numbered-list.fodt"));
    return {
      ok: true,
      status: 200,
      arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length),
    };
  }
  return { ok: false, status: 404 };
};

require(path.join(here, ".smoke/grind_web.js"));

const byId = (id) => document.getElementById(id);
const settle = async () => {
  for (let i = 0; i < 20; i += 1) await new Promise((resolve) => setTimeout(resolve, 10));
};

(async () => {
  await settle();
  check(requested.includes(`../dav/files/${name}`), "the URL is fetched as it is, still encoded");
  check(!document.documentElement.hasAttribute("data-opening"), "the opening mark is off");
  check(posted.every(({ origin }) => origin === "http://localhost"), "only this page's origin is told");
  const types = posted.map(({ message }) => message.type);

  if (outcome === "opens") {
    const opened = posted.find(({ message }) => message.type === "grind:opened");
    check(opened && opened.message.name === "My report.fodt", `opened, its name decoded: ${JSON.stringify(opened?.message)}`);
    check(opened && opened.message.kind === "text", "as a text document");
    check(byId("welcome").hidden, "#welcome is hidden");
    check(byId("name").textContent.includes("My report"), `the header says its name: ${byId("name").textContent}`);
    check(!types.includes("grind:dirty"), "nothing to undo yet, so no dirty message");
    check(byId("undo").disabled, "#undo is disabled with nothing to undo");
    byId("page").dispatchEvent(
      new dom.window.KeyboardEvent("keydown", { key: "x", bubbles: true, cancelable: true })
    );
    await settle();
    const dirty = posted.filter(({ message }) => message.type === "grind:dirty");
    check(dirty.length === 1 && dirty[0].message.dirty === true, `an edit makes it dirty: ${JSON.stringify(dirty.map((d) => d.message))}`);
    check(!byId("undo").disabled, "#undo is enabled once there is something to undo");
  } else {
    const failure = posted.find(({ message }) => message.type === "grind:failed");
    check(failure && /404/.test(failure.message.message), `failed, with why: ${JSON.stringify(failure?.message)}`);
    check(!byId("welcome").hidden, "#welcome is shown");
    check(dom.window.getComputedStyle(byId("welcome")).display !== "none", "and not hidden by the opening mark");
    check(/404/.test(byId("message").textContent), `#message says why: ${byId("message").textContent}`);
    check(!types.includes("grind:opened"), "nothing claims it opened");
  }

  if (failed) {
    console.error(`${failed} check(s) failed`);
    process.exit(1);
  }
  console.log("embedded: all checks passed");
  process.exit(0);
})();
