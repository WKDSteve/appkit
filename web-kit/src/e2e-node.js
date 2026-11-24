// What the browser tests of every desktop app share on the Node side (the Playwright setup, not the page): taking a
// temp folder away again, ending a child, a free port, and the pairing of the hub the tests run against.
//
// Nothing here knows an app. What differs between apps (the binary of the hub, its environment, its origin) comes in as
// arguments. For the page side of the tests see `e2e.js`.

import { execFileSync } from 'node:child_process';
import { rmSync } from 'node:fs';
import { createServer } from 'node:net';

/**
 * Removes `path` (a folder with everything in it, or a file); one that is not there is fine. Retried, because
 * `maxRetries` of `fs.rmSync` does not cover every error Windows gives for a held file; the last error is thrown.
 */
export async function removeTree(path, options = {}) {
  const tries = options.tries ?? 50;
  const pauseMs = options.pauseMs ?? 100;
  const rm = options.rm ?? ((p) => rmSync(p, { recursive: true, force: true }));
  let last;
  for (let i = 0; i < tries; i++) {
    try {
      rm(path);
      return;
    } catch (error) {
      last = error;
    }
    await new Promise((r) => setTimeout(r, pauseMs));
  }
  const reason = last instanceof Error ? last.message : String(last);
  throw new Error(`cannot remove ${path}: ${reason} (a process of the tests still holds a file in it?)`);
}

/** Ends `child` and waits until it has ended (at most `timeoutMs`), so that its files are no longer held. */
export async function stopChild(child, timeoutMs = 10_000) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const ended = new Promise((resolve) => child.once('exit', () => resolve()));
  child.kill();
  let timer;
  const late = new Promise((resolve) => {
    timer = setTimeout(resolve, timeoutMs);
  });
  await Promise.race([ended, late]);
  clearTimeout(timer);
}

/** A port nobody listens on now: the system's pick for port 0 on the loopback. */
export function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address !== null ? address.port : 0;
      server.close(() => (port >= 1024 ? resolve(port) : reject(new Error(`no free port (got ${port})`))));
    });
  });
}

/** A pairing link, as `<app> link` prints it: the first line of its output, which must start with the hub's origin. */
export function pairingLink(config, hub) {
  const out = execFileSync(hub.exe, ['link'], { env: hub.env(config), encoding: 'utf8' });
  const link = out.trim().split(/\r?\n/)[0] ?? '';
  if (!link.startsWith(`${hub.origin}/#pair=`)) throw new Error(`unexpected link: ${link}`);
  return link;
}

/**
 * A session token, got the way the page gets one. The hub allows few exchanges a minute (that is the point of the
 * limit), so the tests share one session and only the pairing tests go through a link.
 */
export async function sessionToken(config, hub) {
  const code = pairingLink(config, hub).split('#pair=')[1] ?? '';
  const response = await fetch(`${hub.origin}/auth/exchange`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Origin: hub.origin },
    body: JSON.stringify({ code }),
  });
  if (!response.ok) throw new Error(`exchange refused: ${response.status}`);
  return (await response.json()).token;
}
