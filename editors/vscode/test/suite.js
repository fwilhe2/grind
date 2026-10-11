// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The extension, in VS Code, against the real `grind lsp` — run by test/run.sh, which starts
// VS Code with this file as its test runner, a fresh profile, no other extension installed and
// the folder *untrusted*: the setting the first session in VS Code was in.
//
// Each check asks VS Code itself, through the same commands its own editor uses
// (`vscode.executeCompletionItemProvider` and friends), so a pass means a person typing would
// see the same thing. A failure throws, and VS Code exits non-zero.

'use strict';

const assert = require('assert');
const path = require('path');
const vscode = require('vscode');

const folder = () => vscode.workspace.workspaceFolders[0].uri.fsPath;

/** Ask until the answer is good, because the server starts while the first request waits. */
async function eventually(what, ask, good, seconds = 30) {
    const until = Date.now() + seconds * 1000;
    let last;
    while (Date.now() < until) {
        last = await ask();
        if (good(last)) {
            return last;
        }
        await new Promise((resolve) => setTimeout(resolve, 250));
    }
    throw new Error(`${what}: never happened; last answer ${JSON.stringify(last)?.slice(0, 400)}`);
}

async function open(name) {
    const document = await vscode.workspace.openTextDocument(path.join(folder(), name));
    await vscode.window.showTextDocument(document);
    return document;
}

const text = (documentation) =>
    typeof documentation === 'string' ? documentation : documentation?.value ?? '';

async function run() {
    const results = [];
    const check = async (name, body) => {
        await body();
        results.push(name);
        console.log(`ok\t${name}`);
    };

    await check('the folder is untrusted, as a fresh one is', async () => {
        assert.strictEqual(vscode.workspace.isTrusted, false);
    });

    await check('a .grind is grind, even untrusted, and shows every formula\'s value', async () => {
        const quote = await open('quote.grind');
        assert.strictEqual(quote.languageId, 'grind');
        const range = new vscode.Range(0, 0, quote.lineCount, 0);
        const hints = await eventually(
            'inlay hints in quote.grind',
            () => vscode.commands.executeCommand('vscode.executeInlayHintProvider', quote.uri, range),
            (hints) => Array.isArray(hints) && hints.length >= 20,
        );
        const labels = hints.map((h) => (typeof h.label === 'string' ? h.label : h.label.map((p) => p.value).join('')));
        assert.ok(labels.includes('→ £62.00'), labels.slice(0, 5).join(', '));
        assert.deepStrictEqual(vscode.languages.getDiagnostics(quote.uri), [], 'it lints clean');

        const line = quote.getText().split('\n').findIndex((l) => l.includes('cell G14'));
        const hovers = await vscode.commands.executeCommand(
            'vscode.executeHoverProvider',
            quote.uri,
            new vscode.Position(line, 10),
        );
        const shown = hovers.flatMap((h) => h.contents.map(text)).join('\n');
        assert.ok(shown.includes('Quote.G14'), shown);
    });

    await check('after `s.` a script is offered a sheet\'s methods, documented', async () => {
        const model = await open('model.rhai');
        assert.strictEqual(model.languageId, 'rhai');
        const at = new vscode.Position(1, 2);
        const list = await eventually(
            'completion after s.',
            () => vscode.commands.executeCommand('vscode.executeCompletionItemProvider', model.uri, at, '.'),
            (list) => list?.items?.some((i) => i.label === 'push' || i.label?.label === 'push'),
        );
        const push = list.items.find((i) => i.label === 'push' || i.label?.label === 'push');
        assert.ok(text(push.documentation).length > 20, `push is documented: ${JSON.stringify(push.documentation)}`);
        assert.ok(!list.items.some((i) => i.label === 'sheet'), 'and no free function after a dot');

        const hovers = await vscode.commands.executeCommand(
            'vscode.executeHoverProvider',
            model.uri,
            new vscode.Position(0, 10),
        );
        const shown = hovers.flatMap((h) => h.contents.map(text)).join('\n');
        assert.ok(shown.includes('sheet(name'), shown);
    });

    await check('a syntax error is underlined where the compiler says', async () => {
        const broken = await open('broken.rhai');
        const found = await eventually(
            'diagnostics in broken.rhai',
            async () => vscode.languages.getDiagnostics(broken.uri),
            (d) => d.length === 1,
        );
        assert.strictEqual(found[0].range.start.line, 2);
    });

    console.log(`${results.length} checks passed`);
}

module.exports = { run };
