// The device key of a page that a phone installs (appkit-hub `devices.rs`). Pairing happens once: the page makes an
// ECDSA P-256 key pair that WebCrypto keeps non-extractable (a script of the page can sign with it, nobody can read it
// out or copy it), sends the public half with the one-time pairing code, and keeps the key and the id the hub gave it
// in IndexedDB of its origin. Every start after that asks the hub for a challenge, signs it together with the origin,
// and gets a fresh session; no code again. A device the PC revoked gets no session: then the page pairs again.
//
// A second key pair, made at the same pairing, confirms a trigger (a route at the level `trigger`, which starts
// something on the PC): it signs that one request (method, path, body) with a fresh challenge (`confirmHeader`). The
// page asks for it only after the person tapped to confirm, never by itself. Unlike a phone app's key it is not bound
// to the phone's biometrics or lock (a page has no such key): it keeps a stolen session token from triggering anything,
// and it makes each trigger one deliberate request; it does not prove a person was there (see SECURITY.md).
//
// Nothing here is a secret that leaves the device: the private keys cannot be exported, the public keys and the id
// are not secret, and the session token stays in memory.

import { ApiError, failure, PLAIN } from './client';

/** What the page keeps: the id the hub gave and the key pair (the private key non-extractable). */
export interface DeviceRecord {
  id: string;
  keys: CryptoKeyPair;
  /** The key that confirms a trigger; missing on a device paired before pages had one (it pairs again to trigger). */
  confirmKeys?: CryptoKeyPair;
}

/** Where the record is kept (IndexedDB in the page, memory in the tests). */
export interface DeviceRecordStore {
  load(): Promise<DeviceRecord | null>;
  save(record: DeviceRecord): Promise<void>;
  clear(): Promise<void>;
}

const ID = /^[0-9a-f]{32}$/;
const TOKEN = /^[0-9a-f]{32,128}$/;

/** The message a device signs: the same as `devices::login_message` of the hub, byte for byte. */
export function loginMessage(origin: string, device: string, challenge: string): string {
  return `appkit-device-login/1\n${origin}\n${device}\n${challenge}`;
}

/** What the confirmation key signs: the same as `devices::confirm_message` of the hub, byte for byte. */
export function confirmMessage(
  origin: string,
  device: string,
  challenge: string,
  method: string,
  pathAndQuery: string,
  bodySha256: string,
): string {
  return `appkit-device-confirm/1\n${origin}\n${device}\n${challenge}\n${method} ${pathAndQuery}\n${bodySha256}`;
}

function hex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** A new key pair; the private key cannot be exported. */
export function makeKeys(subtle: SubtleCrypto): Promise<CryptoKeyPair> {
  return subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']) as Promise<CryptoKeyPair>;
}

/** The public key as the hub takes it: SEC1 uncompressed (65 bytes), lower-case hex. */
export async function publicKeyHex(subtle: SubtleCrypto, keys: CryptoKeyPair): Promise<string> {
  return hex(await subtle.exportKey('raw', keys.publicKey));
}

/** The signature of the login message, as the hub takes it: P1363 (r and s, 64 bytes), lower-case hex. */
export async function signLogin(
  subtle: SubtleCrypto,
  keys: CryptoKeyPair,
  origin: string,
  device: string,
  challenge: string,
): Promise<string> {
  const data = new TextEncoder().encode(loginMessage(origin, device, challenge));
  return hex(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, keys.privateKey, data));
}

async function postJson(fetchFn: typeof fetch, base: string, path: string, body: unknown): Promise<Record<string, unknown>> {
  const response = await fetchFn(`${base}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    ...PLAIN,
  });
  if (!response.ok) throw await failure(response);
  const doc = (await response.json()) as unknown;
  if (typeof doc !== 'object' || doc === null) throw new ApiError(502, 'bad_reply', 'not an object');
  return doc as Record<string, unknown>;
}

function tokenOf(doc: Record<string, unknown>): string {
  const token = doc['token'];
  if (typeof token !== 'string' || !TOKEN.test(token)) throw new ApiError(502, 'bad_reply', 'no token');
  return token;
}

export interface DeviceDeps {
  fetchFn: typeof fetch;
  subtle: SubtleCrypto;
  store: DeviceRecordStore;
  /** The origin of the page (`location.origin`): it is signed, so a signature made for another page opens nothing. */
  origin: string;
  /** Where the hub is, empty for the page's own origin. */
  base?: string;
}

/**
 * Pair this device with a one-time code: a new key pair, the public half and `name` to the hub, the id and the keys
 * kept. Returns a first session token. Any record kept before is replaced only when the hub said yes.
 */
export async function pairDevice(deps: DeviceDeps, code: string, name: string): Promise<string> {
  const keys = await makeKeys(deps.subtle);
  const confirmKeys = await makeKeys(deps.subtle);
  const key = await publicKeyHex(deps.subtle, keys);
  const confirmKey = await publicKeyHex(deps.subtle, confirmKeys);
  const doc = await postJson(deps.fetchFn, deps.base ?? '', '/auth/device/pair', { code, name, key, confirm_key: confirmKey });
  const id = doc['device'];
  if (typeof id !== 'string' || !ID.test(id)) throw new ApiError(502, 'bad_reply', 'no device id');
  const token = tokenOf(doc);
  await deps.store.save({ id, keys, confirmKeys });
  return token;
}

/** The device has no confirmation key (paired before pages had one): to trigger, it pairs again. */
export class NoConfirmKey extends Error {
  constructor() {
    super('this device has no confirmation key; pair it again to trigger');
    this.name = 'NoConfirmKey';
  }
}

async function bodyBytes(body: string | Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
  const bytes = typeof body === 'string' ? new TextEncoder().encode(body) : body;
  const copy = new Uint8Array(new ArrayBuffer(bytes.byteLength));
  copy.set(bytes);
  return copy;
}

/**
 * The confirmation of one trigger, for the header `x-device-confirm`: a fresh challenge and the signature of the
 * confirmation key over `confirmMessage` of exactly this request. Call it only after the person confirmed on the page;
 * send the request with exactly this method, path (with its query) and body, once. `NoConfirmKey` when the device was
 * paired without one; a refused challenge (a revoked device) is the hub's `ApiError`.
 */
export async function confirmHeader(
  deps: DeviceDeps,
  method: string,
  pathAndQuery: string,
  body: string | Uint8Array,
): Promise<string> {
  const record = await deps.store.load();
  if (record === null) throw new ApiError(401, 'unpaired', 'no device');
  if (!record.confirmKeys) throw new NoConfirmKey();
  const asked = await postJson(deps.fetchFn, deps.base ?? '', '/auth/device/challenge', { device: record.id });
  const challenge = asked['challenge'];
  if (typeof challenge !== 'string' || !/^[0-9a-f]{64}$/.test(challenge)) {
    throw new ApiError(502, 'bad_reply', 'no challenge');
  }
  const digest = hex(await deps.subtle.digest('SHA-256', await bodyBytes(body)));
  const message = confirmMessage(deps.origin, record.id, challenge, method.toUpperCase(), pathAndQuery, digest);
  const signature = hex(
    await deps.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, record.confirmKeys.privateKey, new TextEncoder().encode(message)),
  );
  return `${challenge}.${signature}`;
}

/** Can this device confirm a trigger (it was paired with a confirmation key)? */
export async function canConfirm(store: DeviceRecordStore): Promise<boolean> {
  return (await store.load())?.confirmKeys !== undefined;
}

/** What a start finds: a session, no paired device, or a device the hub does not take any more. */
export type LoginResult = { kind: 'session'; token: string } | { kind: 'unpaired' } | { kind: 'refused' };

/**
 * A session for the device kept here: ask a challenge, sign it, open a session. `unpaired` when nothing is kept;
 * `refused` when the hub does not take the device (it was revoked, or the hub is another one): the record is cleared
 * then, so the page asks for pairing. A hub that cannot be reached is an error, not a refusal: the record stays.
 */
export async function deviceLogin(deps: DeviceDeps): Promise<LoginResult> {
  const record = await deps.store.load();
  if (record === null) return { kind: 'unpaired' };
  const base = deps.base ?? '';
  const asked = await postJson(deps.fetchFn, base, '/auth/device/challenge', { device: record.id });
  const challenge = asked['challenge'];
  if (typeof challenge !== 'string' || !/^[0-9a-f]{64}$/.test(challenge)) {
    throw new ApiError(502, 'bad_reply', 'no challenge');
  }
  const signature = await signLogin(deps.subtle, record.keys, deps.origin, record.id, challenge);
  try {
    return { kind: 'session', token: tokenOf(await postJson(deps.fetchFn, base, '/auth/device/session', { device: record.id, challenge, signature })) };
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      await deps.store.clear();
      return { kind: 'refused' };
    }
    throw error;
  }
}

function isRecord(value: unknown): value is DeviceRecord {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as { id?: unknown; keys?: unknown; confirmKeys?: unknown };
  const pair = (k: unknown): boolean => {
    const p = k as { privateKey?: unknown; publicKey?: unknown } | undefined;
    return (
      typeof CryptoKey !== 'undefined' &&
      p?.privateKey instanceof CryptoKey &&
      p.publicKey instanceof CryptoKey &&
      p.privateKey.extractable === false
    );
  };
  // A confirmation key, when there is one, holds to the same rule; a record kept before there was one has none.
  return typeof v.id === 'string' && ID.test(v.id) && pair(v.keys) && (v.confirmKeys === undefined || pair(v.confirmKeys));
}

function request<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

/**
 * The record in IndexedDB of this origin (a `CryptoKey` is kept as it is: still non-extractable when read back). A key
 * that comes back extractable, or anything else of the wrong shape, counts as no record.
 */
export function indexedDbDeviceStore(factory: IDBFactory | undefined, name = 'appkit-device'): DeviceRecordStore {
  const open = (): Promise<IDBDatabase> | null => {
    if (!factory) return null;
    const r = factory.open(name, 1);
    r.onupgradeneeded = () => r.result.createObjectStore('device');
    return request(r);
  };
  const run = async <R>(mode: IDBTransactionMode, work: (s: IDBObjectStore) => IDBRequest<R>): Promise<R | null> => {
    const opening = open();
    if (!opening) return null;
    const db = await opening;
    try {
      return await request(work(db.transaction('device', mode).objectStore('device')));
    } finally {
      db.close();
    }
  };
  return {
    async load() {
      try {
        const value = await run('readonly', (s) => s.get('record'));
        return isRecord(value) ? value : null;
      } catch {
        return null;
      }
    },
    async save(record) {
      await run('readwrite', (s) => s.put(record, 'record'));
    },
    async clear() {
      await run('readwrite', (s) => s.delete('record'));
    },
  };
}
