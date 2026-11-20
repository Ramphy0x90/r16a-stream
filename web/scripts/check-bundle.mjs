// Fails when the initial JavaScript of the production build exceeds 200 KB gzipped, and when
// hls.js ends up in an initial chunk. Angular budgets measure raw bytes only, so this is the
// gzip check that SPEC 15 requires. Run after ng build.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';

const LIMIT = 200 * 1024;
const DIST = new URL('../dist/r16a-stream-web/browser/', import.meta.url).pathname;

const html = readFileSync(join(DIST, 'index.html'), 'utf8');
// Initial chunks are the module scripts and modulepreload links in index.html.
const initial = new Set();
for (const m of html.matchAll(/(?:src|href)="([^"]+\.js)"/g)) {
  initial.add(m[1]);
}
if (initial.size === 0) {
  console.error('no initial scripts found in index.html, is this a production build?');
  process.exit(1);
}
let total = 0;
for (const file of initial) {
  const content = readFileSync(join(DIST, file));
  const gz = gzipSync(content, { level: 9 }).length;
  total += gz;
  if (/hls\.js|class Hls\b|hls\.light/.test(content.toString('utf8'))) {
    console.error(`${file} contains hls.js, which must only load on the player route`);
    process.exit(1);
  }
  console.log(`${file.padEnd(40)} ${(gz / 1024).toFixed(1)} KB gzipped`);
}
console.log(`initial total ${(total / 1024).toFixed(1)} KB gzipped, limit ${LIMIT / 1024} KB`);
const lazy = readdirSync(DIST).filter((f) => f.endsWith('.js') && !initial.has(f)).length;
console.log(`${lazy} lazy chunks`);
if (total > LIMIT) {
  process.exit(1);
}
