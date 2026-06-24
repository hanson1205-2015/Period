const vscode = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');
const path = require('path');
const fs = require('fs');

let client = undefined;

function findServerExecutable(context) {
    // Respect explicit user configuration first.
    const config = vscode.workspace.getConfiguration('period');
    const configured = config.get('languageServerPath');
    if (configured && fs.existsSync(configured)) {
        return configured;
    }

    const isWindows = process.platform === 'win32';
    const commandName = isWindows ? 'period.exe' : 'period';

    // Prefer the sibling compiler executable installed by the Windows installer.
    const extRoot = context.extensionPath;
    const sibling = path.join(extRoot, '..', commandName);
    if (fs.existsSync(sibling)) {
        return sibling;
    }

    // Fallback: look for the executable on PATH.
    return commandName;
}

function startClient(context) {
    const serverExecutable = findServerExecutable(context);
    const serverOptions = {
        run: { command: serverExecutable, args: ['--lsp'], transport: TransportKind.stdio },
        debug: { command: serverExecutable, args: ['--lsp'], transport: TransportKind.stdio }
    };

    const clientOptions = {
        documentSelector: [{ scheme: 'file', language: 'period' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.period')
        }
    };

    client = new LanguageClient('period', 'Period Language Server', serverOptions, clientOptions);
    client.start();
}

function activate(context) {
    startClient(context);
}

function deactivate() {
    if (!client) {
        return undefined;
    }
    return client.stop();
}

module.exports = { activate, deactivate };
