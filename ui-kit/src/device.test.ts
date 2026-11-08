import { describe, expect, it } from 'vitest';
import { deviceLogin, loginMessage, pairDevice, type DeviceDeps, type DeviceRecord, type DeviceRecordStore } from './device';

// Node's WebCrypto, the same API as the browser's.
const subtle = globalThis.crypto.subtle;
const ORIGIN = 'http://127.0.0.1:17615';

function memory(): DeviceRecordStore & { value: DeviceRecord | null } {
  const box = {
    value: null as DeviceRecord | null,
    load: async () => box.value,
    save: async (r: DeviceRecord) => {
      box.value = r;
    },
    clear: async () => {
      box.value = null;
    },
  };
  return box;
}

function fromHex(text: string): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(new ArrayBuffer(text.length / 2));
  for (let i = 0; i < out.length; i += 1) out[i] = parseInt(text.slice(2 * i, 2 * i + 2), 16);
  return out;
}

/** A hub as `devices.rs` is: it keeps public keys, hands out challenges, checks signatures with WebCrypto. */
function fakeHub() {
  const keys = new Map<string, CryptoKey>();
  const challenges = new Map<string, string>();
  const codes = new Set(['c'.repeat(32)]);
  let n = 0;
  const json = (doc: unknown, status = 200) => new Response(JSON.stringify(doc), { status, headers: { 'Content-Type': 'application/json' } });
  const fetchFn = (async (url: string, init: RequestInit) => {
    if (init.credentials !== 'omit' || init.cache !== 'no-store') throw new Error('not a plain request');
    const body = JSON.parse(String(init.body)) as Record<string, string>;
    if (url.endsWith('/auth/device/pair')) {
      if (!codes.delete(body['code']!)) return json({ error: { code: 'bad_code', message: 'no' } }, 401);
      const id = (++n).toString(16).padStart(32, '0');
      keys.set(id, await subtle.importKey('raw', fromHex(body['key']!), { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify']));
      return json({ device: id, token: 'a'.repeat(64) });
    }
    if (url.endsWith('/auth/device/challenge')) {
      const challenge = (n + 7).toString(16).padStart(64, '0');
      challenges.set(challenge, body['device']!);
      return json({ challenge });
    }
    if (url.endsWith('/auth/device/session')) {
      const device = challenges.get(body['challenge']!);
      challenges.delete(body['challenge']!);
      const key = keys.get(body['device']!);
      const ok =
        device === body['device'] &&
        key !== undefined &&
        (await subtle.verify(
          { name: 'ECDSA', hash: 'SHA-256' },
          key,
          fromHex(body['signature']!),
          new TextEncoder().encode(loginMessage(ORIGIN, body['device']!, body['challenge']!)),
        ));
      return ok ? json({ device: body['device'], token: 'b'.repeat(64) }) : json({ error: { code: 'bad_signature', message: 'no' } }, 401);
    }
    return json({}, 404);
  }) as unknown as typeof fetch;
  return { fetchFn, keys };
}

function deps(fetchFn: typeof fetch, store: DeviceRecordStore, origin = ORIGIN): DeviceDeps {
  return { fetchFn, subtle, store, origin };
}

describe('device keys', () => {
  it('pairs once, keeps a non-extractable key, and logs in with a signed challenge after that', async () => {
    const hub = fakeHub();
    const store = memory();
    expect(await pairDevice(deps(hub.fetchFn, store), 'c'.repeat(32), 'Phone')).toBe('a'.repeat(64));
    expect(store.value?.id).toMatch(/^[0-9a-f]{32}$/);
    expect(store.value?.keys.privateKey.extractable).toBe(false);
    await expect(subtle.exportKey('pkcs8', store.value!.keys.privateKey)).rejects.toThrow();
    expect(await deviceLogin(deps(hub.fetchFn, store))).toEqual({ kind: 'session', token: 'b'.repeat(64) });
    // Again, as often as the app starts.
    expect((await deviceLogin(deps(hub.fetchFn, store))).kind).toBe('session');
  });

  it('is unpaired without a record, and keeps nothing when the code is refused', async () => {
    const hub = fakeHub();
    const store = memory();
    expect(await deviceLogin(deps(hub.fetchFn, store))).toEqual({ kind: 'unpaired' });
    await expect(pairDevice(deps(hub.fetchFn, store), 'd'.repeat(32), 'Phone')).rejects.toMatchObject({ status: 401 });
    expect(store.value).toBeNull();
  });

  it('forgets the record when the hub refuses the device (revoked), so the page pairs again', async () => {
    const hub = fakeHub();
    const store = memory();
    await pairDevice(deps(hub.fetchFn, store), 'c'.repeat(32), 'Phone');
    hub.keys.clear();
    expect(await deviceLogin(deps(hub.fetchFn, store))).toEqual({ kind: 'refused' });
    expect(store.value).toBeNull();
  });

  it('signs the origin: a signature made for another origin is refused', async () => {
    const hub = fakeHub();
    const store = memory();
    await pairDevice(deps(hub.fetchFn, store), 'c'.repeat(32), 'Phone');
    expect((await deviceLogin(deps(hub.fetchFn, store, 'http://127.0.0.1:1'))).kind).toBe('refused');
  });

  it('keeps the record when the hub cannot be reached', async () => {
    const hub = fakeHub();
    const store = memory();
    await pairDevice(deps(hub.fetchFn, store), 'c'.repeat(32), 'Phone');
    const offline = (async () => {
      throw new TypeError('Failed to fetch');
    }) as unknown as typeof fetch;
    await expect(deviceLogin(deps(offline, store))).rejects.toThrow('Failed to fetch');
    expect(store.value).not.toBeNull();
  });

  it('writes the login message the hub checks, byte for byte', () => {
    expect(loginMessage('http://127.0.0.1:1', 'ab', 'cd')).toBe('appkit-device-login/1\nhttp://127.0.0.1:1\nab\ncd');
  });
});
