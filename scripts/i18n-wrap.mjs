// One-off helper that wrapped the UI's plain text in t(...). Kept for reference and for wrapping new components:
//   node scripts/i18n-wrap.mjs src/NewComponent.tsx
// It wraps JSX text that is the only child of an element, and the placeholder/title/aria-label/alt attributes.
// Anything else (text mixed with expressions, strings in objects) it reports for hand editing.
import { readFileSync, writeFileSync } from 'node:fs';
import ts from 'typescript';

const ATTRS = new Set(['placeholder', 'title', 'aria-label', 'alt']);
const norm = (s) => s.replace(/\s+/g, ' ').trim();
const lit = (s) => JSON.stringify(s);

for (const file of process.argv.slice(2)) {
  const text = readFileSync(file, 'utf8');
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const edits = [];
  const report = [];
  const where = (n) => `${file}:${sf.getLineAndCharacterOfPosition(n.getStart()).line + 1}`;

  const visit = (n) => {
    if (ts.isJsxElement(n)) {
      const kids = n.children;
      const textKids = kids.filter((k) => ts.isJsxText(k) && norm(k.text));
      const onlyElements = kids.every((k) => ts.isJsxText(k) || ts.isJsxElement(k) || ts.isJsxSelfClosingElement(k));
      if (textKids.length === 1 && onlyElements) {
        // One piece of text, possibly next to icons or inputs: translate just that piece.
        const k = textKids[0];
        const lead = /^\s/.test(k.text) && kids.indexOf(k) > 0 ? ' ' : '';
        edits.push({ start: k.getStart(), end: k.getEnd(), text: `${lead}{t(${lit(norm(k.text))})}` });
      } else if (textKids.length) {
        for (const k of textKids) report.push(`${where(k)}  mixed text: ${norm(k.text)}`);
      }
    }
    if (ts.isJsxAttribute(n) && ATTRS.has(n.name.getText()) && n.initializer && ts.isStringLiteral(n.initializer)) {
      edits.push({ start: n.initializer.getStart(), end: n.initializer.getEnd(), text: `{t(${lit(n.initializer.text)})}` });
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  if (!edits.length) { for (const r of report) console.log(r); continue; }
  let out = text;
  for (const e of edits.sort((a, b) => b.start - a.start)) out = out.slice(0, e.start) + e.text + out.slice(e.end);
  if (!/from "\.\/i18n"/.test(out)) {
    const lines = out.split('\n');
    const last = lines.map((l, i) => (l.startsWith('import ') ? i : -1)).reduce((a, b) => Math.max(a, b), -1);
    lines.splice(last + 1, 0, 'import { t } from "./i18n";');
    out = lines.join('\n');
  }
  writeFileSync(file, out);
  console.log(`${file}: wrapped ${edits.length}`);
  for (const r of report) console.log(r);
}
