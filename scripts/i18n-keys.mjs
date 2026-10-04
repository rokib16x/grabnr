// Prints every English text that the UI asks to translate: t("..."), tn("one", "other", n) and msg("...").
//   node scripts/i18n-keys.mjs            (one per line)
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

export function keys(dir = 'src') {
  const out = new Set();
  const files = readdirSync(dir).filter((f) => /\.(ts|tsx)$/.test(f) && !/\.test\./.test(f) && f !== 'mock.ts');
  for (const f of files) {
    const text = readFileSync(join(dir, f), 'utf8');
    const sf = ts.createSourceFile(f, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const lit = (n) => (n && (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) ? n.text : null);
    const visit = (n) => {
      if (ts.isCallExpression(n) && ts.isIdentifier(n.expression)) {
        const name = n.expression.text;
        if (name === 't' || name === 'msg') { const k = lit(n.arguments[0]); if (k) out.add(k); }
        if (name === 'tn') for (const i of [0, 1]) { const k = lit(n.arguments[i]); if (k) out.add(k); }
      }
      ts.forEachChild(n, visit);
    };
    visit(sf);
  }
  return [...out].sort((a, b) => a.localeCompare(b));
}

if (import.meta.url === `file://${process.argv[1]}`) console.log(keys().join('\n'));
