// Fails when the translation files do not all have exactly the same keys, or a value is empty.
// English is the reference. Run with `npm run check:i18n`.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const DIR = new URL('../public/i18n/', import.meta.url).pathname;

function flatten(obj, prefix = '', out = new Map()) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v !== null && typeof v === 'object') {
      flatten(v, key, out);
    } else {
      out.set(key, v);
    }
  }
  return out;
}

const files = readdirSync(DIR).filter((f) => f.endsWith('.json'));
const maps = new Map(
  files.map((f) => [f, flatten(JSON.parse(readFileSync(join(DIR, f), 'utf8')))]),
);
const reference = maps.get('en.json');
if (!reference) {
  console.error('en.json is missing');
  process.exit(1);
}
let problems = 0;
for (const [file, map] of maps) {
  for (const key of reference.keys()) {
    if (!map.has(key)) {
      console.error(`${file} is missing ${key}`);
      problems++;
    }
  }
  for (const [key, value] of map) {
    if (!reference.has(key)) {
      console.error(`${file} has ${key}, which en.json does not`);
      problems++;
    }
    if (typeof value !== 'string' || value.trim() === '') {
      console.error(`${file} has an empty value for ${key}`);
      problems++;
    }
  }
}
if (problems > 0) {
  process.exit(1);
}
console.log(`i18n ok, ${files.length} languages, ${reference.size} keys`);
