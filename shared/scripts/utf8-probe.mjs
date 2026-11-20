#!/usr/bin/env node
/**
 * Sends requests with invalid UTF-8 in the path and the query string, raw and percent-encoded,
 * then checks that Garage is still healthy. Used to test Garage directly and through each
 * proxy in front of it (DECISIONS 055).
 *
 * Usage
 *   node shared/scripts/utf8-probe.mjs <base url> [garage health url]
 *   node shared/scripts/utf8-probe.mjs http://127.0.0.1:4200 http://127.0.0.1:3903/health
 */

import { connect } from 'node:net';

const TARGETS = [
  ['path, percent-encoded', '/r16a-stream-media/%FF%FE'],
  ['path, raw bytes', Buffer.from([...Buffer.from('/r16a-stream-media/'), 0xff, 0xfe])],
  ['query value, percent-encoded', '/r16a-stream-media/a?x=%FF%FE'],
  ['query key, percent-encoded', '/r16a-stream-media/a?%FF%FE=1'],
  ['query value, raw bytes', Buffer.from([...Buffer.from('/r16a-stream-media/a?x='), 0xff, 0xfe])],
  ['overlong utf-8 in uploadId', '/r16a-stream-uploads/a?partNumber=1&uploadId=%C0%AF'],
  ['bucket name, percent-encoded', '/%FF%FE?list-type=2'],
];

function send(base, target) {
  const url = new URL(base);
  return new Promise((resolve) => {
    const socket = connect(Number(url.port || 80), url.hostname);
    const chunks = [];
    socket.setTimeout(5000, () => {
      socket.destroy();
      resolve('timeout');
    });
    socket.on('error', (e) => resolve(`error ${e.code}`));
    socket.on('data', (d) => chunks.push(d));
    socket.on('end', () => {
      const line = Buffer.concat(chunks).toString('latin1').split('\r\n')[0] ?? '';
      resolve(line.slice(9, 12) || 'closed without a response');
    });
    socket.on('connect', () => {
      const head = Buffer.from('GET ');
      const tail = Buffer.from(` HTTP/1.1\r\nHost: ${url.host}\r\nConnection: close\r\n\r\n`);
      socket.write(Buffer.concat([head, Buffer.from(target), tail]));
    });
  });
}

const [base, health] = process.argv.slice(2);
if (!base) {
  console.error('usage: utf8-probe.mjs <base url> [garage health url]');
  process.exit(2);
}
for (const [label, target] of TARGETS) {
  console.log(`${label.padEnd(32)} ${await send(base, target)}`);
}
if (health) {
  const res = await fetch(health).catch(() => null);
  console.log(`garage health afterwards        ${res ? res.status : 'unreachable'}`);
  if (!res || res.status !== 200) {
    process.exit(1);
  }
}
