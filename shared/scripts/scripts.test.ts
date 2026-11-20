/**
 * Unit tests for the repository tooling. Run with `npm run test:scripts`.
 */

import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

// @ts-expect-error the checker is plain JavaScript without type declarations
import { findings, extractComments, proseOf } from './check-comment-style.mjs';
import { CSS_OUT, DART_OUT, buildCss, buildDart } from './tokens-build.ts';
import { measure } from './contrast-check.ts';
import { contrastRatio, loadTokens, scaleColour, toHex } from './tokens-lib.ts';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const tokens = loadTokens(join(ROOT, 'packages/design-tokens/tokens.json'));
const WHITE = { h: 0, s: 0, l: 100, a: 1 };

test('brand base is exactly #10b981', () => {
  assert.equal(toHex(scaleColour(tokens, 'brand.500')), '#10b981');
});

test('39 percent lightness renders as #10b77f, the reason the base uses 39.4', () => {
  assert.equal(toHex({ h: 160, s: 84, l: 39, a: 1 }), '#10b77f');
});

test('white on the brand base fails AA, as SPEC 14.1 states', () => {
  const ratio = contrastRatio(WHITE, scaleColour(tokens, 'brand.500'));
  assert.ok(Math.abs(ratio - 2.54) < 0.01, `ratio ${ratio}`);
});

test('every declared pairing passes in both themes', () => {
  const failing = measure(tokens).filter((r) => r.ratio < r.min);
  assert.deepEqual(failing, []);
});

test('committed generated files match the token source', () => {
  assert.equal(readFileSync(CSS_OUT, 'utf8'), buildCss(tokens));
  assert.equal(readFileSync(DART_OUT, 'utf8'), buildDart(tokens));
});

test('comment checker flags a colon in prose', () => {
  const out = findings(join(ROOT, 'x.rs'), '// Note: this is wrong\nlet a = 1;\n');
  assert.equal(out.length, 1);
});

test('comment checker flags an em dash', () => {
  const out = findings(join(ROOT, 'x.ts'), '/** One thing — another */\n');
  assert.equal(out.length, 1);
});

test('comment checker ignores code spans, URLs, paths and strings', () => {
  const src = [
    '/// Uses `Cache-Control: no-store` like https://example.org/a:b and crate::config.',
    'let url = "http://localhost:3900"; // fine',
    '# syntax=docker/dockerfile:1.7',
  ].join('\n');
  assert.deepEqual(findings(join(ROOT, 'x.rs'), src), []);
  assert.equal(proseOf(' see `a: b`').trim(), 'see');
});

test('comment checker reads hash comments only when separated from code', () => {
  const yaml = 'image: postgres:15 # pinned tag\nurl: http://a#frag\n';
  const comments = extractComments(yaml, { line: ['#'], block: false, html: false });
  assert.deepEqual(
    comments.map((c: { text: string }) => c.text.trim()),
    ['pinned tag'],
  );
});
