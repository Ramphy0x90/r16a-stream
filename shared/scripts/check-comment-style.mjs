#!/usr/bin/env node
/**
 * Enforces the repository comment rule, no colon and no em dash inside comment sentences.
 *
 * It extracts comments from source files and flags a colon or an em dash that is not part of
 * inline code, a URL, a path like `crate::module`, or a tool directive. Exits 1 with one line
 * per finding.
 *
 * Usage
 *   node shared/scripts/check-comment-style.mjs            whole repository
 *   node shared/scripts/check-comment-style.mjs a.rs b.ts  only those files
 */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { extname, basename, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');

const SKIP_DIRS = new Set([
  '.git',
  'node_modules',
  'target',
  'dist',
  '.angular',
  'build',
  '.dart_tool',
  '.fvm',
  '.sqlx',
  'Pods',
  '.gradle',
  '.idea',
  'coverage',
  'playwright-report',
  'test-results',
  '.scratch',
]);

/** Generated or tool-owned files whose comments we do not author. */
const SKIP_FILES = [
  /^mobile\/lib\/core\/api\/generated\//,
  /^mobile\/lib\/l10n\/generated\//,
  /^web\/src\/app\/core\/api\/schema\.d\.ts$/,
  /^mobile\/(android|ios)\//,
  /^package-lock\.json$/,
];

const LINE_STYLES = {
  '.rs': ['//'],
  '.ts': ['//'],
  '.mjs': ['//'],
  '.js': ['//'],
  '.dart': ['//'],
  '.scss': ['//'],
  '.sh': ['#'],
  '.yml': ['#'],
  '.yaml': ['#'],
  '.toml': ['#'],
  '.sql': ['--'],
  '.conf': ['#'],
  '.template': ['#'],
};
const BLOCK_EXTS = new Set(['.rs', '.ts', '.mjs', '.js', '.scss', '.css']);
const HTML_EXTS = new Set(['.html']);

function styleFor(path) {
  const name = basename(path);
  if (name === 'Dockerfile' || name.endsWith('.Dockerfile')) {
    return { line: ['#'], block: false, html: false };
  }
  const ext = extname(path);
  return { line: LINE_STYLES[ext] ?? [], block: BLOCK_EXTS.has(ext), html: HTML_EXTS.has(ext) };
}

/** Returns comment fragments with their 1-based line numbers. String literals are skipped. */
export function extractComments(source, style) {
  const out = [];
  const lines = source.split('\n');
  let inBlock = false;
  let blockEnd = '*/';
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    let j = 0;
    let quote = null;
    while (j < line.length) {
      if (inBlock) {
        const end = line.indexOf(blockEnd, j);
        const text = end === -1 ? line.slice(j) : line.slice(j, end);
        out.push({ line: i + 1, text });
        if (end === -1) {
          j = line.length;
        } else {
          inBlock = false;
          j = end + blockEnd.length;
        }
        continue;
      }
      const ch = line[j];
      if (quote) {
        if (ch === '\\') {
          j += 2;
          continue;
        }
        if (ch === quote) {
          quote = null;
        }
        j++;
        continue;
      }
      if (ch === '"' || ch === '`' || (ch === "'" && !style.line.includes('#'))) {
        quote = ch;
        j++;
        continue;
      }
      if (style.block && line.startsWith('/*', j)) {
        inBlock = true;
        blockEnd = '*/';
        j += 2;
        continue;
      }
      if (style.html && line.startsWith('<!--', j)) {
        inBlock = true;
        blockEnd = '-->';
        j += 4;
        continue;
      }
      const marker = style.line.find((m) => line.startsWith(m, j));
      if (marker) {
        const before = j === 0 ? ' ' : line[j - 1];
        // A hash or dash glued to a word is data, as in a URL fragment or a YAML value.
        if (marker === '//' || /\s/.test(before)) {
          out.push({ line: i + 1, text: line.slice(j + marker.length) });
          break;
        }
      }
      j++;
    }
  }
  return out;
}

const DIRECTIVES = [
  /^!/, // shebang or inner doc marker handled below
  /^\s*syntax=/,
  /^\s*@ts-/,
  /^\s*eslint-/,
  /^\s*prettier-ignore/,
  /^\s*yaml-language-server/,
  /^\s*language=/,
  /^\s*ignore_for_file/,
];

/** Strips everything that may legitimately contain a colon and returns what is left. */
export function proseOf(text) {
  let t = text.replace(/^[/!*#\s-]+/, '');
  if (DIRECTIVES.some((d) => d.test(t))) {
    return '';
  }
  t = t
    .replace(/`[^`]*`/g, '')
    .replace(/\b[a-z][a-z0-9+.-]*:\/\/\S+/gi, '')
    .replace(/\S*::\S*/g, '')
    .replace(/\[[^\]]*\]\([^)]*\)/g, '');
  return t;
}

export function findings(path, source) {
  const rel = relative(ROOT, path);
  const style = styleFor(path);
  if (style.line.length === 0 && !style.block && !style.html) {
    return [];
  }
  const result = [];
  for (const c of extractComments(source, style)) {
    if (c.line === 1 && c.text.startsWith('!')) {
      continue;
    }
    const prose = proseOf(c.text);
    if (prose.includes(':') || prose.includes('—')) {
      result.push(`${rel}:${c.line}: ${c.text.trim()}`);
    }
  }
  return result;
}

function walk(dir, files) {
  for (const entry of readdirSync(dir)) {
    if (SKIP_DIRS.has(entry)) {
      continue;
    }
    const full = join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) {
      walk(full, files);
    } else {
      files.push(full);
    }
  }
}

function main() {
  const args = process.argv.slice(2);
  const files = [];
  if (args.length > 0) {
    files.push(...args.map((a) => join(process.cwd(), a)));
  } else {
    walk(ROOT, files);
  }
  const all = [];
  for (const f of files) {
    const rel = relative(ROOT, f);
    if (SKIP_FILES.some((r) => r.test(rel))) {
      continue;
    }
    all.push(...findings(f, readFileSync(f, 'utf8')));
  }
  if (all.length > 0) {
    console.error(all.join('\n'));
    console.error(`\n${all.length} comment(s) contain a colon or an em dash, rephrase them (CLAUDE.md, comment rule).`);
    process.exit(1);
  }
  console.log('comment style ok');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
