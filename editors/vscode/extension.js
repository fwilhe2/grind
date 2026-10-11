// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// grind's VS Code client — doc/init.md I6.
//
// All it does is start `grind lsp` and introduce VS Code to it. Every answer — completion,
// hover, diagnostics, the values shown beside formulas — is the server's, and the server's are
// the core's, so nothing here knows what a cell or a script is. That is the point of I5 being a
// language server rather than an extension: Helix and Neovim get the same answers from the same
// binary, and this file stays small enough to read.
//
// Plain JavaScript, so the extension builds without a compiler: `npm install` and `vsce
// package` are the whole of it (editors/vscode/package.sh).

'use strict';

const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

/** @type {LanguageClient | undefined} */
let client;

/**
 * The documents the server answers for. A `.grind` by its name too, not only by language: a
 * project `grind init` set up associates `*.grind` with KDL, and the server should follow the
 * file rather than the association.
 */
const DOCUMENTS = [
    { scheme: 'file', language: 'grind' },
    { scheme: 'file', pattern: '**/*.grind' },
    { scheme: 'file', language: 'rhai' },
    { scheme: 'untitled', language: 'grind' },
    { scheme: 'untitled', language: 'rhai' },
];

async function start() {
    // `grind.path` is a restricted setting (package.json): in an untrusted folder VS Code reads
    // it from the user's settings only, so a folder cannot name the program that runs here.
    const command = vscode.workspace.getConfiguration('grind').get('path') || 'grind';
    client = new LanguageClient(
        'grind',
        'grind',
        { command, args: ['lsp'] },
        { documentSelector: DOCUMENTS },
    );
    try {
        await client.start();
    } catch (error) {
        client = undefined;
        const message = error instanceof Error ? error.message : String(error);
        const choice = await vscode.window.showErrorMessage(
            `grind: could not start \`${command} lsp\` (${message}). ` +
                'Install grind, or set "grind.path" to where it is.',
            'Open Settings',
        );
        if (choice) {
            await vscode.commands.executeCommand('workbench.action.openSettings', 'grind.path');
        }
    }
}

async function restart() {
    if (client) {
        const running = client;
        client = undefined;
        await running.stop();
    }
    await start();
}

/** @param {vscode.ExtensionContext} context */
async function activate(context) {
    context.subscriptions.push(
        vscode.commands.registerCommand('grind.restartServer', restart),
        vscode.workspace.onDidChangeConfiguration((event) => {
            if (event.affectsConfiguration('grind.path')) {
                restart();
            }
        }),
    );
    await start();
}

function deactivate() {
    return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
