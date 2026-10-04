// Builds the extension for each browser into a folder and a zip:
//   node extension/tools/build.mjs [outDir]      (default: extension/dist)
//
// Chrome, Brave and Edge use the manifest as it is. Firefox needs a different background declaration, no `key`,
// and its own add-on id, so its manifest is derived from the same source here.
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const FILES = ['background.js', 'lib.js', 'popup.html', 'popup.js', 'options.html', 'options.js', 'icons'];

export function chromeManifest(m) {
  return structuredClone(m);
}

export function firefoxManifest(m) {
  const f = structuredClone(m);
  delete f.key; // Chrome-only: pins the extension id
  // Firefox runs the background as an event page made of scripts; it has no service workers.
  f.background = { scripts: [m.background.service_worker], type: 'module' };
  f.browser_specific_settings = {
    gecko: { id: 'grabnr@grabnr.invalid', strict_min_version: '140.0', data_collection_permissions: { required: ['none'] } },
    gecko_android: { strict_min_version: '142.0' },
  };
  return f;
}

export function build(outDir = join(root, 'dist')) {
  const manifest = JSON.parse(readFileSync(join(root, 'manifest.json'), 'utf8'));
  rmSync(outDir, { recursive: true, force: true });
  const made = {};
  for (const [name, make] of [['chrome', chromeManifest], ['firefox', firefoxManifest]]) {
    const dir = join(outDir, name);
    mkdirSync(dir, { recursive: true });
    for (const f of FILES) cpSync(join(root, f), join(dir, f), { recursive: true });
    writeFileSync(join(dir, 'manifest.json'), JSON.stringify(make(manifest), null, 2) + '\n');
    const zip = join(outDir, `grabnr-${name}.zip`);
    execFileSync('zip', ['-qr', zip, '.', '-x', '*.DS_Store'], { cwd: dir });
    made[name] = { dir, zip };
  }
  return made;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const made = build(process.argv[2] ? resolve(process.argv[2]) : undefined);
  for (const [n, p] of Object.entries(made)) console.log(`${n}: ${p.zip}`);
}
