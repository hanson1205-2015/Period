const vscodeTextmate = require('vscode-textmate');
const vscodeOniguruma = require('vscode-oniguruma');
const fs = require('fs');
const path = require('path');

const wasmBin = fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm')).buffer;

class SimpleOnigurumaAdapter {
  constructor() {
    this._onigLib = vscodeOniguruma.loadWASM(wasmBin).then(() => ({
      createOnigScanner(patterns) { return new vscodeOniguruma.OnigScanner(patterns); },
      createOnigString(s) { return vscodeOniguruma.createOnigString(s); }
    }));
  }
  getOnigLib() { return this._onigLib; }
}

async function main() {
  const adapter = new SimpleOnigurumaAdapter();
  const registry = new vscodeTextmate.Registry({
    onigLib: adapter,
    loadGrammar: (scopeName) => {
      if (scopeName === 'source.period') {
        const content = fs.readFileSync(path.join(__dirname, 'syntaxes', 'period.tmLanguage.json'));
        return Promise.resolve(vscodeTextmate.parseRawGrammar(content.toString(), 'period.tmLanguage.json'));
      }
      return Promise.resolve(null);
    }
  });

  const grammar = await registry.loadGrammar('source.period');
  const lines = ['show "{a}".', '{{{{}}}}'];
  let ruleStack = vscodeTextmate.INITIAL;
  for (const line of lines) {
    const lineTokens = grammar.tokenizeLine(line, ruleStack);
    console.log(`\nLINE: ${line}`);
    for (let i = 0; i < lineTokens.tokens.length; i++) {
      const t = lineTokens.tokens[i];
      const text = line.substring(t.startIndex, t.endIndex);
      console.log(`  [${t.startIndex}:${t.endIndex}] "${text}" scopes: ${t.scopes.join(' ')}`);
    }
    ruleStack = lineTokens.ruleStack;
  }
}

main().catch(console.error);
