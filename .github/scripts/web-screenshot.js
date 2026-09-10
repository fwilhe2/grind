// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Screenshot `ui_web/dist`'s page, once per document type, in a real headless
// browser. `ui_web/smoke.js` drives the same page in jsdom, but jsdom has no
// layout engine and no canvas — every rectangle it reports is zero, so it can
// check the wiring but never show what the page looks like. This is the browser
// half that smoke test deliberately does not attempt.
//
//   node web-screenshot.js <base-url> <out-dir> <name>=<doc-query> [<name>=<doc-query> ...]
//
// Each `<doc-query>` is the value of the page's own `?doc=` parameter (a filename
// already sitting next to index.html, per scripts/run.sh).

const { chromium } = require("playwright");
const path = require("path");

async function main() {
  const [, , baseUrl, outDir, ...pairs] = process.argv;
  if (!baseUrl || !outDir || pairs.length === 0) {
    console.error(
      "usage: web-screenshot.js <base-url> <out-dir> <name>=<doc> [...]",
    );
    process.exit(2);
  }

  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });

  for (const pair of pairs) {
    const eq = pair.indexOf("=");
    const name = pair.slice(0, eq);
    const doc = pair.slice(eq + 1);

    await page.goto(`${baseUrl}/?doc=${encodeURIComponent(doc)}`, {
      waitUntil: "networkidle",
    });
    // `#name` starts empty and is set once `App::open_bytes` has run and the
    // shell has refreshed (ui_web/src/lib.rs) — the same readiness the page
    // itself has no other way to announce.
    await page.waitForFunction(
      () => document.getElementById("name").textContent.trim().length > 0,
      { timeout: 15000 },
    );
    // The fetch resolving is the readiness signal; the next paint is not
    // observable from outside, so this gives it one frame's grace.
    await page.waitForTimeout(300);

    const out = path.join(outDir, `${name}.png`);
    await page.screenshot({ path: out });
    console.log(`wrote ${out}`);
  }

  await browser.close();
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
