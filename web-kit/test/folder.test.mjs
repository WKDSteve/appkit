// The folder a page works in: the picker, the permission after a restart, the folders used before, and the IndexedDB
// they are kept in, one database per app. A fake window, fake folder handles and a small fake IndexedDB: no browser.
//
//   node --test test/folder.test.mjs        (from web-kit/)

import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  displayName,
  grantAccess,
  idbKeyValue,
  LOOK_WAIT_MS,
  MAX_RECENT,
  oneLine,
  permissionOf,
  pickerSupport,
  pickFolder,
  RecentFolders,
  within,
} from '../src/folder.js';
import { fakeIdb } from './fake-idb.mjs';

function domError(name, message = name) {
  return Object.assign(new Error(message), { name });
}

/** A folder handle. `perm` is what queryPermission says, `onRequest` what requestPermission does. */
function handle(name, { perm = 'granted', onRequest, same = [], asked = [] } = {}) {
  const h = {
    kind: 'directory',
    name,
    perm,
    async queryPermission(descriptor) {
      asked.push(['query', descriptor]);
      if (h.perm instanceof Error) throw h.perm;
      return h.perm;
    },
    async requestPermission(descriptor) {
      asked.push(['request', descriptor]);
      if (onRequest) return onRequest(h);
      h.perm = 'granted';
      return 'granted';
    },
    async isSameEntry(other) {
      return same.includes(other.name);
    },
    async *values() {},
  };
  return h;
}

function memoryKv() {
  const map = new Map();
  return { map, get: async (k) => map.get(k), put: async (k, v) => void map.set(k, v) };
}

const SECURE = (showDirectoryPicker) => ({ showDirectoryPicker, isSecureContext: true });

test('which windows can choose a folder', () => {
  assert.equal(pickerSupport({}), 'missing');
  assert.equal(pickerSupport({ showDirectoryPicker: 3 }), 'missing');
  assert.equal(pickerSupport({ showDirectoryPicker: () => {}, isSecureContext: false }), 'insecure');
  assert.equal(pickerSupport({ showDirectoryPicker: () => {}, isSecureContext: true }), 'ok');
  assert.equal(pickerSupport({ showDirectoryPicker: () => {} }), 'ok');
});

test('the picker is asked with the id and the mode of the app, and each way it fails is told apart', async () => {
  const asked = [];
  const h = handle('v');
  assert.deepEqual(await pickFolder(SECURE(async (o) => (asked.push(o), h)), { id: 'app-a', mode: 'readwrite' }), { kind: 'picked', handle: h });
  await pickFolder(SECURE(async (o) => (asked.push(o), h)), { id: 'app-b', mode: 'read' });
  assert.deepEqual(asked, [{ id: 'app-a', mode: 'readwrite' }, { id: 'app-b', mode: 'read' }]);
  const opts = { id: 'x', mode: 'read' };
  assert.deepEqual(await pickFolder(SECURE(async () => { throw domError('AbortError'); }), opts), { kind: 'cancelled' });
  for (const name of ['SecurityError', 'NotAllowedError']) {
    const r = await pickFolder(SECURE(async () => { throw domError(name, 'blocked by policy'); }), opts);
    assert.equal(r.kind, 'blocked', name);
    assert.match(r.message, /blocked by policy/);
  }
  const odd = await pickFolder(SECURE(async () => { throw new Error('line1\nline2\u0007' + 'x'.repeat(500)); }), opts);
  assert.equal(odd.kind, 'failed');
  assert.ok(odd.message.length <= 200 && !/[\u0000-\u001f]/.test(odd.message), 'one short line');
  assert.equal((await pickFolder({}, opts)).kind, 'failed');
  assert.equal(oneLine('a\tb'), 'a b');
  assert.equal(displayName('a\nb'), 'a b');
  assert.equal(displayName(7), '');
});

test('the permission of a stored folder, and asking for it again, in the mode the app asks for', async () => {
  for (const state of ['granted', 'prompt', 'denied']) {
    assert.equal(await permissionOf(handle('v', { perm: state }), 'readwrite'), state);
  }
  assert.equal(await permissionOf(handle('v', { perm: domError('NotFoundError') }), 'read'), 'gone');
  assert.equal(await permissionOf(handle('v', { perm: domError('Whatever') }), 'read'), 'denied', 'a failure is no permission');
  assert.equal(await grantAccess(handle('v', { perm: 'prompt' }), 'read'), 'granted');
  assert.equal(await grantAccess(handle('v', { onRequest: () => 'denied' }), 'read'), 'denied');
  assert.equal(await grantAccess(handle('v', { onRequest: () => { throw domError('NotFoundError'); } }), 'read'), 'gone');
  const asked = [];
  const h = handle('v', { asked });
  await permissionOf(h, 'read');
  await grantAccess(h, 'readwrite');
  assert.deepEqual(asked, [['query', { mode: 'read' }], ['request', { mode: 'readwrite' }]]);
});

test('a browser that never answers queryPermission is taken to want to be asked, within the limit', async () => {
  const silent = handle('v');
  silent.queryPermission = () => new Promise(() => {});
  // A guard of the test's own (not the kit's `within`): without the limit the answer is 'no answer', not a hang.
  let guard;
  const noAnswer = new Promise((resolve) => {
    guard = setTimeout(() => resolve('no answer'), 1000);
  });
  assert.equal(await Promise.race([permissionOf(silent, 'read', 20), noAnswer]), 'prompt');
  clearTimeout(guard);
  // The default limit is the kit's, a few seconds: short enough for a page that waits on it.
  assert.ok(LOOK_WAIT_MS > 0 && LOOK_WAIT_MS <= 5000, `${LOOK_WAIT_MS}`);
  // An answer in time is the answer.
  assert.equal(await permissionOf(handle('v', { perm: 'denied' }), 'read', 1000), 'denied');
});

test('within: the answer, the fallback after the limit, a rejection in time', async () => {
  assert.equal(await within(Promise.resolve('yes'), 1000, 'late'), 'yes');
  assert.equal(await within(new Promise(() => {}), 10, 'late'), 'late');
  await assert.rejects(within(Promise.reject(new Error('no')), 1000, 'late'), /no/);
});

test('the folders used before: newest first, one entry per folder, a limit, and a list that is not trusted', async () => {
  const kv = memoryKv();
  let n = 0;
  const recent = new RecentFolders(kv, { newId: () => `id${++n}` });
  assert.deepEqual(await recent.list(), []);
  // The stored handle answers `isSameEntry`: it knows the handle named 'a-again' as itself.
  const a = handle('a', { same: ['a-again'] });
  const b = handle('b');
  await recent.remember(a, 100);
  await recent.remember(b, 200);
  assert.deepEqual((await recent.list()).map((r) => r.name), ['b', 'a']);

  // Another handle for a folder that is in the list moves it up and keeps its id.
  const moved = await recent.remember(handle('a-again'), 300);
  assert.equal(moved.id, 'id1', 'the id of the folder stays');
  assert.deepEqual((await recent.list()).map((r) => r.name), ['a-again', 'b']);

  // A handle that is not the same entry is another folder, also under the same name.
  await recent.remember(handle('b'), 350);
  assert.deepEqual((await recent.list()).map((r) => r.name), ['b', 'a-again', 'b']);
  await recent.forget((await recent.list())[0].id);

  for (let i = 0; i < 8; i++) await recent.remember(handle(`f${i}`), 1000 + i);
  const names = (await recent.list()).map((r) => r.name);
  assert.equal(names.length, MAX_RECENT);
  assert.equal(names[0], 'f7');

  await recent.forget((await recent.list())[0].id);
  assert.equal((await recent.list()).length, MAX_RECENT - 1);
  assert.equal((await new RecentFolders(memoryKv(), { max: 2 }).remember(handle('x'), 1)).name, 'x');
});

test('what is stored is checked: an entry that is no folder is dropped, a failing store is an empty list', async () => {
  const kv = memoryKv();
  const good = handle('good\u0001name');
  // A directory that lacks one of the functions this module calls on it, one at a time.
  const lacking = (fn) => {
    const h = { ...handle(`no ${fn}`) };
    delete h[fn];
    return h;
  };
  kv.map.set('recent', [
    null,
    3,
    { id: 'no-handle' },
    { id: '', handle: good, lastUsed: 1 },
    { id: 'x', handle: { kind: 'file', name: 'f' }, lastUsed: 2 },
    { id: 'bare', handle: { kind: 'directory', name: 'no functions' }, lastUsed: 2 },
    ...['values', 'queryPermission', 'requestPermission'].map((fn) => ({ id: fn, handle: lacking(fn), lastUsed: 2 })),
    { id: 'y', handle: good, lastUsed: 'soon' },
    { id: 'y', handle: good, lastUsed: 9 },
    { id: 'ok', handle: handle('ok'), lastUsed: 5 },
    { id: 'nan', handle: handle('n'), lastUsed: Number.NaN },
    { id: 'inf', handle: handle('i'), lastUsed: Number.POSITIVE_INFINITY },
  ]);
  const list = await new RecentFolders(kv).list();
  // A time that is no finite number counts as 0: it sorts last and keeps the order of the others.
  assert.deepEqual(
    list.map((r) => [r.id, r.name, r.lastUsed]),
    [['ok', 'ok', 5], ['y', 'good name', 0], ['nan', 'n', 0], ['inf', 'i', 0]],
    'one entry per id, the first wins; only full directory handles are kept',
  );
  kv.map.set('recent', 'not a list');
  assert.deepEqual(await new RecentFolders(kv).list(), []);
  assert.deepEqual(await new RecentFolders({ get: async () => { throw new Error('io'); }, put: async () => {} }).list(), []);
});

test('the IndexedDB store keeps a value under a key, in the database it is named for', async () => {
  const idb = fakeIdb();
  const kv = idbKeyValue(idb, 'app-a', 'handles');
  assert.equal(await kv.get('recent'), undefined);
  await kv.put('recent', [{ id: 'a', n: 1 }]);
  assert.deepEqual(await kv.get('recent'), [{ id: 'a', n: 1 }]);
  assert.deepEqual(idb.dump(), { 'app-a': { handles: { recent: [{ id: 'a', n: 1 }] } } });
  assert.throws(() => idbKeyValue(idb), /name of the database/);
  assert.throws(() => idbKeyValue(idb, ''), /name of the database/);
});

test('a failed open is tried again, an aborted write is an error', async () => {
  const idb = fakeIdb({ failOpen: 1 });
  const kv = idbKeyValue(idb, 'app');
  await assert.rejects(kv.get('k'), /open failed/);
  assert.equal(await kv.get('k'), undefined, 'the second try opens again');
  assert.equal(idb.opens(), 2);
  const quota = idbKeyValue(fakeIdb({ abortWrites: true }), 'app');
  await assert.rejects(quota.put('k', 1), /quota/);
});

test('two apps in the same file:// origin keep their folders in databases of their own', async () => {
  // One origin: every local HTML file shares it, and with it IndexedDB.
  const origin = fakeIdb();
  const tasks = new RecentFolders(idbKeyValue(origin, 'tasks-web'), { newId: () => 'tasks-id' });
  const testing = new RecentFolders(idbKeyValue(origin, 'testing-web'), { newId: () => 'testing-id' });

  await tasks.remember(handle('tasks-vault'), 10);
  assert.deepEqual(await testing.list(), [], 'the other app sees no folder of the first');
  await testing.remember(handle('testing-export'), 20);
  assert.deepEqual((await tasks.list()).map((r) => [r.id, r.name]), [['tasks-id', 'tasks-vault']]);
  assert.deepEqual((await testing.list()).map((r) => [r.id, r.name]), [['testing-id', 'testing-export']]);

  // Forgetting in one app leaves the other's folder where it is.
  await testing.forget('testing-id');
  assert.deepEqual(await testing.list(), []);
  assert.deepEqual((await tasks.list()).map((r) => r.id), ['tasks-id']);

  // What the origin holds: two databases, each with only its own app's entries.
  const dump = origin.dump();
  assert.deepEqual(Object.keys(dump).sort(), ['tasks-web', 'testing-web']);
  assert.deepEqual(dump['tasks-web'].handles.recent.map((e) => e.id), ['tasks-id']);
  assert.deepEqual(dump['testing-web'].handles.recent, []);
});
