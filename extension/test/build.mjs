// Checks that the Chrome and Firefox builds are what each browser needs. Run: node test/build.mjs
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { build } from '../tools/build.mjs';

const out = mkdtempSync(join(tmpdir(), 'grabnr-ext-'));
const made = build(out);
const read = (p) => JSON.parse(readFileSync(join(p, 'manifest.json'), 'utf8'));

const chrome = read(made.chrome.dir);
assert.equal(chrome.manifest_version, 3);
assert.equal(chrome.background.service_worker, 'background.js');
assert.ok(chrome.key, 'Chrome keeps the key that pins its extension id');

const ff = read(made.firefox.dir);
assert.equal(ff.manifest_version, 3);
assert.deepEqual(ff.background, { scripts: ['background.js'], type: 'module' });
assert.equal(ff.background.service_worker, undefined, 'Firefox has no service workers');
assert.equal(ff.key, undefined, 'the key is Chrome-only');
assert.ok(ff.browser_specific_settings.gecko.id.includes('@'));
assert.ok(ff.host_permissions.includes('http://127.0.0.1:17653/*'));

for (const d of [made.chrome.dir, made.firefox.dir]) {
  for (const f of ['background.js', 'lib.js', 'popup.html', 'options.html', 'icons/icon128.png']) {
    assert.ok(existsSync(join(d, f)), `${f} is missing from ${d}`);
  }
}
assert.ok(existsSync(made.chrome.zip) && existsSync(made.firefox.zip));

const sf = read(made.safari.dir);
assert.ok(!sf.permissions.includes('downloads') && !sf.permissions.includes('notifications'), 'Safari has neither API');
assert.deepEqual(sf.background, { scripts: ['background.js'] });
const bg = readFileSync(join(made.safari.dir, 'background.js'), 'utf8');
assert.ok(!/^\s*(import|export)\b/m.test(bg), 'the Safari background must not use module syntax');
assert.ok(bg.includes('async function addDownload') && bg.includes('function createMenu'), 'lib.js and background.js are both in it');
new Function(bg); // it must at least parse as a plain script
console.log('build: all assertions passed');
