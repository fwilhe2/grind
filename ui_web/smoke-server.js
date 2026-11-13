// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The page against a real `grind serve` (doc/server.md): smoke.js is the page on its own, this is
// the same module with a server behind it. Run by smoke.sh, which starts the server on a free
// port over a git repository holding budget.fods and passes both in the environment.

const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const { JSDOM, VirtualConsole } = require("jsdom");

const here = __dirname;
const port = process.env.GRIND_SMOKE_PORT;
const store = process.env.GRIND_SMOKE_DIR;
const html = fs
  .readFileSync(path.join(here, "index.html"), "utf8")
  .replace(/<script type="module">[\s\S]*?<\/script>/, "");

const virtualConsole = new VirtualConsole();
virtualConsole.sendTo(console, { omitJSDOMErrors: true });
virtualConsole.on("jsdomError", (error) => {
  if (!/not implemented/i.test(error.message)) console.error(error);
});
const dom = new JSDOM(html, { pretendToBeVisual: true, url: "http://localhost/", virtualConsole });
global.window = dom.window;
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (key in global) continue;
  try {
    global[key] = dom.window[key];
  } catch {}
}
global.document = dom.window.document;

const downloads = [];
globalThis.URL.createObjectURL = (blob) => {
  downloads.push(blob);
  return "blob:smoke";
};
globalThis.URL.revokeObjectURL = () => {};
dom.window.HTMLAnchorElement.prototype.click = function () {
  downloads.at(-1).name = this.download;
};

// The page's relative requests reach the server that, in a browser, would have served it.
const nodeFetch = globalThis.fetch;
const sent = [];
dom.window.fetch = async (url, init = {}) => {
  url = String(url);
  if (/^[a-z]+:\/\//i.test(url)) throw new Error(`fetch left the page's own files: ${url}`);
  sent.push(`${init.method ?? "GET"} ${url}`);
  if (!url.startsWith("api/")) return { ok: false, status: 404 };
  return nodeFetch(`http://127.0.0.1:${port}/${url}`, init);
};

// The answers a person would give the page's questions, set before each one is asked.
let answer = null;
let agree = true;
dom.window.prompt = () => answer;
dom.window.confirm = () => agree;

require(path.join(here, ".smoke/grind_web.js"));

const byId = (id) => document.getElementById(id);
const frame = () => new Promise((resolve) => dom.window.requestAnimationFrame(resolve));
const shown = () => document.querySelector("td.active").textContent;
const press = (key, modifiers = {}) =>
  byId("surface").dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers })
  );
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
const command = async (query) => {
  palette("k", { ctrlKey: true });
  paletteType(query);
  paletteEnter();
  await frame();
};
const library = () =>
  [...document.querySelectorAll("#welcome-server-list button")].map((b) => b.textContent);
const message = () => byId("message").textContent;
const onDisk = (name) => fs.readFileSync(path.join(store, name), "utf8");
const commits = () =>
  execFileSync("git", ["-C", store, "log", "--format=%s"], { encoding: "utf8" }).trim().split("\n");

// Requests are real ones, so a check waits for what it is about rather than for a frame.
const until = async (what, label) => {
  for (let n = 0; n < 200; n++) {
    if (what()) return true;
    await new Promise((resolve) => setTimeout(resolve, 25));
    await frame();
  }
  console.log(`FAIL ${label} (timed out)`);
  failures += 1;
  return false;
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

(async () => {
  await until(() => !byId("welcome-server").hidden, "the probe finds the server");
  check("the page asked the server what it is", sent[0], "GET api/v1/hello");
  await until(() => library().length > 0, "the library arrives");
  check("the welcome pane lists the server's documents", library(), ["budget.fods"]);
  check("and says where documents go", byId("welcome-hint").textContent.includes("server"), true);

  document.querySelector("#welcome-server-list button").click();
  await until(() => !byId("surface").hidden, "a library document opens");
  check("under its name on the server", byId("name").textContent, "budget.fods");

  // B1, a number on one line of the file.
  press("ArrowRight");
  await enter("7");
  byId("save").click();
  await until(() => message().includes("on the server"), "Save writes back");
  check("Save writes it back", message(), "Saved budget.fods on the server");
  check("into the file", onDisk("budget.fods").includes('office:value="7"'), true);
  check("as a commit", commits()[0], "Save budget.fods");
  check("of one line", execFileSync("git", ["-C", store, "show", "--numstat", "--format=", "HEAD"],
    { encoding: "utf8" }).trim(), "1\t1\tbudget.fods");
  check("and nothing was downloaded", downloads.length, 0);

  // Somebody else saves first: another program writing the file is that somebody.
  fs.writeFileSync(path.join(store, "budget.fods"), onDisk("budget.fods").replace('office:value="7"', 'office:value="8"'));
  press("ArrowUp");
  await enter("9");
  agree = false;
  byId("save").click();
  await until(() => message().includes("not saved"), "a stale save is refused");
  check("a stale save is refused, and the file left alone", onDisk("budget.fods").includes('office:value="8"'), true);
  agree = true;
  byId("save").click();
  // The file is written before the page hears back, so wait for the page too.
  await until(() => onDisk("budget.fods").includes('office:value="9"') && message().startsWith("Saved"),
    "replacing theirs is a choice");
  check("replacing theirs is a choice", message(), "Saved budget.fods on the server");

  await command("Earlier versions");
  await until(() => paletteRows().some((row) => row.startsWith("Version: ")), "the versions are listed");
  check("the versions are listed, newest first",
    paletteRows().filter((row) => row.startsWith("Version: ")).length >= 2, true);
  palette("Escape");

  answer = "copy.fods";
  await command("Save to the server as");
  await until(() => message().includes("copy.fods"), "save as a new document");
  check("a copy goes up under a new name", byId("name").textContent, "copy.fods");
  check("as a file of its own", fs.existsSync(path.join(store, "copy.fods")), true);

  answer = "renamed.fods";
  await command("Rename on the server");
  await until(() => message().includes("Renamed"), "rename");
  check("it is renamed in place", [fs.existsSync(path.join(store, "copy.fods")), fs.existsSync(path.join(store, "renamed.fods"))], [false, true]);

  await command("Delete from the server");
  await until(() => message().includes("Deleted"), "delete");
  check("and deleted", fs.existsSync(path.join(store, "renamed.fods")), false);
  check("which takes it out of the palette's server verbs",
    (palette("k", { ctrlKey: true }), paletteType("Rename on the server"), paletteRows().length), 0);
  palette("Escape");

  check("every request went to the server under api/v1/, or beside the page",
    sent.every((line) => line.startsWith("GET") || line.includes("api/v1/")), true);

  console.log(failures === 0 ? "\nall server checks passed" : `\n${failures} server check(s) failed`);
  process.exit(failures === 0 ? 0 : 1);
})();
