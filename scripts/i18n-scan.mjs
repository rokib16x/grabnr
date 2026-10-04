// Lists string literals in the UI source that look like text for people and are not wrapped in t(...)/tn(...).
//   node scripts/i18n-scan.mjs src/*.tsx
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const SKIP_ATTRS = new Set(['className', 'type', 'name', 'role', 'key', 'id', 'src', 'href', 'viewBox', 'fill', 'stroke', 'd', 'width', 'height', 'rel', 'target', 'autoComplete', 'inputMode', 'value', 'htmlFor', 'aria-current', 'aria-expanded', 'aria-pressed', 'aria-selected', 'data-tauri-drag-region', 'step', 'min', 'max', 'rows', 'size', 'strokeWidth', 'strokeLinecap', 'strokeLinejoin']);
let found = 0;
for (const file of process.argv.slice(2)) {
  const text = readFileSync(file, 'utf8');
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const inT = (n) => { for (let p = n.parent; p; p = p.parent) if (ts.isCallExpression(p) && ['t', 'tn', 'msg'].includes(p.expression.getText())) return true; return false; };
  const looksHuman = (s) => /[A-Za-z]{3,}/.test(s) && (/\s/.test(s) || /^[A-Z]/.test(s)) && !/^(https?:|\/|#|[a-z]+-[a-z-]+$|[a-z]+:[a-z])/.test(s) && !/^[a-z_]+$/.test(s);
  const visit = (n) => {
    if (ts.isImportDeclaration(n)) return;
    if ((ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n) || ts.isTemplateHead(n)) && !inT(n)) {
      const par = n.parent;
      const attr = ts.isJsxAttribute(par) ? par.name.getText() : ts.isJsxExpression(par) && ts.isJsxAttribute(par.parent) ? par.parent.name.getText() : null;
      const isTypeLit = ts.isLiteralTypeNode(par) || ts.isTypeReferenceNode(par);
      const isKey = (ts.isPropertyAssignment(par) && par.name === n) || ts.isElementAccessExpression(par) || (ts.isCaseClause(par)) || ts.isBinaryExpression(par) && ['===', '!==', '==', '!='].includes(par.operatorToken.getText());
      const isDataAttr = attr && SKIP_ATTRS.has(attr);
      const s = n.text ?? n.getText();
      if (!isTypeLit && !isKey && !isDataAttr && looksHuman(s)) {
        found++;
        console.log(`${file}:${sf.getLineAndCharacterOfPosition(n.getStart()).line + 1}  ${JSON.stringify(s)}`);
      }
    }
    if (ts.isJsxText(n) && n.text.trim() && !inT(n) && /[A-Za-z]{3,}/.test(n.text)) {
      found++;
      console.log(`${file}:${sf.getLineAndCharacterOfPosition(n.getStart()).line + 1}  JSX text ${JSON.stringify(n.text.trim())}`);
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
console.log(found ? `${found} candidate strings` : 'no unwrapped UI strings');
process.exit(found ? 1 : 0);
