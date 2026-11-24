// The Node side of the browser-test helpers, without a hub: the removal that is retried and then fails loudly, the
// child that is waited for, a port that is free, and the pairing with a fake hub binary and a fake exchange route.
//
//   node --test test/e2e-node.test.mjs        (from web-kit/)

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { createServer as netServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { pathToFileURL } from 'node:url';
import { freePort, pairingLink, removeTree, sessionToken, stopChild } from '../src/e2e-node.js';

test('removeTree takes a folder away, and one that is not there is fine', async () => {
  const root = mkdtempSync(join(tmpdir(), 'appkit-web-e2e-node-'));
  writeFileSync(join(root, 'file'), 'x');
  await removeTree(root);
  assert.equal(existsSync(root), false);
  await removeTree(root);
});

test('removeTree retries a failing removal and then throws, never a silent leftover', async () => {
  let calls = 0;
  await removeTree('x', {
    pauseMs: 1,
    rm: () => {
      calls++;
      if (calls < 4) throw new Error('EBUSY');
    },
  });
  assert.equal(calls, 4);

  calls = 0;
  await assert.rejects(
    removeTree('held', {
      tries: 3,
      pauseMs: 1,
      rm: () => {
        calls++;
        throw new Error('EBUSY');
      },
    }),
    /cannot remove held: EBUSY/,
  );
  assert.equal(calls, 3);
});

test('stopChild waits until the child has ended, and an ended one is left alone', async () => {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
  await new Promise((r) => child.once('spawn', r));
  await stopChild(child);
  assert.ok(child.exitCode !== null || child.signalCode !== null);
  await stopChild(child);
});

test('freePort gives a port that can be listened on', async () => {
  const port = await freePort();
  assert.ok(port >= 1024 && port <= 65535);
  await new Promise((resolve, reject) => {
    const server = netServer();
    server.once('error', reject);
    server.listen(port, '127.0.0.1', () => server.close(resolve));
  });
});

/**
 * A hub that prints `output` for `link`. A script cannot be run directly by Node on Windows (no `.cmd` without a shell),
 * so the binary is Node itself and a preload (NODE_OPTIONS) prints and ends before `link` is taken for a file.
 */
function fakeHub(output, origin) {
  const dir = mkdtempSync(join(tmpdir(), 'appkit-web-e2e-node-exe-'));
  const preload = join(dir, 'hub.mjs');
  writeFileSync(preload, `console.log(${JSON.stringify(output)}); console.log(process.env.FAKE_CFG); process.exit(0);
`);
  const options = `--import ${pathToFileURL(preload).href}`;
  return { exe: process.execPath, env: (config) => ({ ...process.env, NODE_OPTIONS: options, FAKE_CFG: config }), origin };
}

test('pairingLink is the first line of the output, and it must start with the origin of the hub', () => {
  const hub = fakeHub('http://127.0.0.1:9/#pair=abc', 'http://127.0.0.1:9');
  assert.equal(pairingLink('cfg', hub), 'http://127.0.0.1:9/#pair=abc');
  assert.throws(() => pairingLink('cfg', { ...hub, origin: 'http://127.0.0.1:10' }), /unexpected link/);
});

test('sessionToken exchanges the code of the link, and a refused exchange is an error', async () => {
  let seen;
  let status = 200;
  const server = createServer((request, response) => {
    let body = '';
    request.on('data', (chunk) => (body += chunk));
    request.on('end', () => {
      seen = { url: request.url, origin: request.headers.origin, body: JSON.parse(body) };
      response.statusCode = status;
      response.end(JSON.stringify({ token: 'tok' }));
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const hub = fakeHub(`${origin}/#pair=code1`, origin);
  try {
    assert.equal(await sessionToken('cfg', hub), 'tok');
    assert.deepEqual(seen, { url: '/auth/exchange', origin, body: { code: 'code1' } });
    status = 429;
    await assert.rejects(sessionToken('cfg', hub), /exchange refused: 429/);
  } finally {
    await new Promise((r) => server.close(r));
  }
});
