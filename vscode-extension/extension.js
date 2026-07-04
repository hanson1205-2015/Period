const vscode = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');
const path = require('path');
const fs = require('fs');

let client = undefined;

// ---------------------------------------------------------------------------
// Rainbow interpolation braces inside Period string literals.
// VS Code's native bracket pair colorization skips brackets inside strings, so
// we compute nesting levels ourselves and apply TextEditor decorations.
// ---------------------------------------------------------------------------

const DEFAULT_BRACKET_COLORS = [
    '#ffd700', // 1 gold
    '#da70d6', // 2 orchid
    '#87cefa', // 3 light sky blue
    '#7cfc00', // 4 lawn green
    '#ff8c00', // 5 dark orange
    '#00bfff'  // 6 deep sky blue
];
const UNEXPECTED_BRACKET_COLOR = '#ff1212';

let interpolationDecorationTypes = [];
let interpolationUnexpectedType = null;
let lastAppliedColorKey = '';

function getBracketHighlightColors() {
    const config = vscode.workspace.getConfiguration('workbench');
    const customizations = config.get('colorCustomizations') || {};
    const colors = [];
    for (let i = 1; i <= 6; i++) {
        const value = customizations[`editorBracketHighlight.foreground${i}`];
        colors.push(value || DEFAULT_BRACKET_COLORS[i - 1]);
    }
    return colors;
}

function getColorKey(colors) {
    return colors.join('|');
}

function ensureDecorationTypes(colors) {
    const key = getColorKey(colors);
    if (key === lastAppliedColorKey && interpolationDecorationTypes.length > 0) {
        return;
    }
    interpolationDecorationTypes.forEach(dt => dt.dispose());
    if (interpolationUnexpectedType) {
        interpolationUnexpectedType.dispose();
    }
    interpolationDecorationTypes = colors.map(color =>
        vscode.window.createTextEditorDecorationType({
            color,
            rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed
        })
    );
    interpolationUnexpectedType = vscode.window.createTextEditorDecorationType({
        color: UNEXPECTED_BRACKET_COLOR,
        rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed
    });
    lastAppliedColorKey = key;
}

function disposeInterpolationDecorations() {
    interpolationDecorationTypes.forEach(dt => dt.dispose());
    interpolationDecorationTypes = [];
    if (interpolationUnexpectedType) {
        interpolationUnexpectedType.dispose();
        interpolationUnexpectedType = null;
    }
    lastAppliedColorKey = '';
}

function findStringBraces(document) {
    const text = document.getText();
    const braces = [];
    const stringRegex = /"([^"\\]|\\.)*"/gs;
    let match;
    while ((match = stringRegex.exec(text)) !== null) {
        const strStart = match.index + 1; // after opening "
        const content = match[0].slice(1, -1);
        for (let i = 0; i < content.length; i++) {
            const ch = content[i];
            const next = content[i + 1];
            if (ch === '\\' && i + 1 < content.length) {
                i++; // skip the escaped character
                continue;
            }
            if (ch === '{' || ch === '}') {
                braces.push({
                    offset: strStart + i,
                    char: ch
                });
            }
        }
    }
    return braces;
}

function computeBraceLevels(braces) {
    const stack = [];
    return braces.map(b => {
        if (b.char === '{') {
            stack.push(b);
            return { ...b, level: stack.length, matched: true };
        }
        if (stack.length > 0) {
            stack.pop();
            return { ...b, level: stack.length + 1, matched: true };
        }
        return { ...b, level: 0, matched: false };
    });
}

function updateInterpolationDecorations(editor) {
    if (!editor || editor.document.languageId !== 'period') {
        return;
    }

    const colors = getBracketHighlightColors();
    ensureDecorationTypes(colors);

    const braces = computeBraceLevels(findStringBraces(editor.document));
    const rangesByLevel = colors.map(() => []);
    const unexpectedRanges = [];

    for (const b of braces) {
        const range = new vscode.Range(
            editor.document.positionAt(b.offset),
            editor.document.positionAt(b.offset + 1)
        );
        if (!b.matched) {
            unexpectedRanges.push(range);
        } else {
            const idx = (b.level - 1) % colors.length;
            rangesByLevel[idx].push(range);
        }
    }

    interpolationDecorationTypes.forEach((dt, i) => {
        editor.setDecorations(dt, rangesByLevel[i]);
    });
    if (interpolationUnexpectedType) {
        editor.setDecorations(interpolationUnexpectedType, unexpectedRanges);
    }
}

function clearInterpolationDecorations(editor) {
    if (!editor) return;
    interpolationDecorationTypes.forEach(dt => editor.setDecorations(dt, []));
    if (interpolationUnexpectedType) {
        editor.setDecorations(interpolationUnexpectedType, []);
    }
}

function registerInterpolationDecorator(context) {
    const updateActive = () => updateInterpolationDecorations(vscode.window.activeTextEditor);

    vscode.window.onDidChangeActiveTextEditor(editor => {
        clearInterpolationDecorations(vscode.window.activeTextEditor);
        updateInterpolationDecorations(editor);
    }, null, context.subscriptions);

    vscode.workspace.onDidChangeTextDocument(event => {
        const editor = vscode.window.activeTextEditor;
        if (editor && editor.document === event.document) {
            updateInterpolationDecorations(editor);
        }
    }, null, context.subscriptions);

    vscode.workspace.onDidChangeConfiguration(event => {
        if (event.affectsConfiguration('workbench.colorCustomizations')) {
            lastAppliedColorKey = '';
            updateActive();
        }
    }, null, context.subscriptions);

    updateActive();
}

function findServerExecutable(context) {
    // Respect explicit user configuration first.
    const config = vscode.workspace.getConfiguration('period');
    const configured = config.get('languageServerPath');
    if (configured && fs.existsSync(configured)) {
        return configured;
    }

    return findExecutable(context, false);
}

function findRunExecutable(context) {
    // Running a file should always go through the wrapper (period / period.exe),
    // even if the language server was configured to use period-core directly.
    return findExecutable(context, true);
}

function findExecutable(context, preferWrapper) {
    const isWindows = process.platform === 'win32';
    const wrapperName = isWindows ? 'period.exe' : 'period';
    const coreName = isWindows ? 'period-core.exe' : 'period-core';

    const config = vscode.workspace.getConfiguration('period');
    const configured = config.get('languageServerPath');
    if (configured && fs.existsSync(configured)) {
        const base = path.basename(configured);
        if (preferWrapper && base.toLowerCase() === coreName.toLowerCase()) {
            const wrapper = path.join(path.dirname(configured), wrapperName);
            if (fs.existsSync(wrapper)) {
                return wrapper;
            }
        }
        return configured;
    }

    const extRoot = context.extensionPath;

    // Prefer a bundled compiler executable inside the extension folder (development layout).
    const bundled = path.join(extRoot, wrapperName);
    if (fs.existsSync(bundled)) {
        return bundled;
    }

    // Prefer the sibling compiler executable installed by the Windows installer.
    const sibling = path.join(extRoot, '..', wrapperName);
    if (fs.existsSync(sibling)) {
        return sibling;
    }

    // Prefer a compiler in the current workspace root (development / source layout).
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (workspaceFolders && workspaceFolders.length > 0) {
        const local = path.join(workspaceFolders[0].uri.fsPath, wrapperName);
        if (fs.existsSync(local)) {
            return local;
        }
    }

    // Fallback: look for the executable on PATH.
    return wrapperName;
}

async function startClient(context) {
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
    await client.start();
}

function runCurrentFile(context) {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'period') {
        vscode.window.showWarningMessage('Open a .period file to run it.');
        return;
    }

    const filePath = editor.document.fileName;
    const executable = findRunExecutable(context);

    // Run through the wrapper command name on PATH for a clean terminal command:
    //   period "<file>"
    // This requires Period to be on PATH (the installer does this). If for some
    // reason the resolved executable is not the wrapper, fall back to the full path.
    const base = path.basename(executable).toLowerCase();
    const commandName = (base === 'period.exe' || base === 'period') ? 'period' : executable;

    const fileArg = JSON.stringify(filePath);
    const command = commandName.includes(' ')
        ? `& ${JSON.stringify(commandName)} ${fileArg}`
        : `${commandName} ${fileArg}`;

    const terminal = vscode.window.terminals.find(t => t.name === 'Period')
        || vscode.window.createTerminal('Period');
    terminal.show();
    terminal.sendText(command, true);
}

async function activate(context) {
    const runCommand = vscode.commands.registerCommand('period.runFile', () => {
        runCurrentFile(context);
    });
    context.subscriptions.push(runCommand);

    registerInterpolationDecorator(context);
    context.subscriptions.push({ dispose: disposeInterpolationDecorations });

    try {
        await startClient(context);
    } catch (err) {
        console.error('Period language server failed to start:', err);
    }
}

function deactivate() {
    disposeInterpolationDecorations();
    if (!client) {
        return undefined;
    }
    return client.stop();
}

module.exports = { activate, deactivate };
