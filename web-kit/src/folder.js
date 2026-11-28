// The folder the person works in, in an app's web variant: choosing it (File System Access API), the folders used
// before (their handles in IndexedDB), and the permission to read them, which the browser asks again after a restart.
//
// Everything that is the browser's comes in by argument (the window, the key-value store), so Node runs this without
// one. The probe of tasks-web (2026-10-28) showed on the workstation what this relies on: the handle survives a reload
// and a restart of Chrome, the permission is `granted` after a reload and `prompt` after a restart, and only a click
// (`requestPermission` inside a user gesture) brings it back.
//
// Every local HTML file shares one origin (file://), and with it IndexedDB and the stored handles with their
// permission. So each app keeps its handles in a database of its own name (`idbKeyValue(factory, name)`), and the
// picker remembers its start folder under an id of the app's own (`pickFolder(win, { id })`).

export const MAX_RECENT = 5;

/** Whether this page can choose a folder at all. A policy that blocks it shows only when the picker is called. */
export function pickerSupport(win) {
  if (typeof win.showDirectoryPicker !== 'function') return 'missing';
  if (win.isSecureContext === false) return 'insecure';
  return 'ok';
}

/** One line of text of an error: no control characters, cut short. */
export function oneLine(error, max = 200) {
  const raw = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  // eslint-disable-next-line no-control-regex
  return raw.replace(/[\u0000-\u001f\u007f-\u009f]+/g, ' ').trim().slice(0, max);
}

function nameOf(error) {
  return typeof error === 'object' && error !== null && typeof error.name === 'string' ? error.name : '';
}

/**
 * Ask the person for a folder. Must run inside a click: the browser refuses the picker without one. `id` is the app's
 * (the picker starts at the folder last chosen under it), `mode` what is asked for.
 */
export async function pickFolder(win, { id, mode }) {
  if (typeof win.showDirectoryPicker !== 'function') return { kind: 'failed', message: 'showDirectoryPicker is missing' };
  try {
    return { kind: 'picked', handle: await win.showDirectoryPicker({ id, mode }) };
  } catch (error) {
    const name = nameOf(error);
    if (name === 'AbortError') return { kind: 'cancelled' };
    // A policy of the organisation, or no click behind the call: the picker is refused, not failed.
    if (name === 'SecurityError' || name === 'NotAllowedError') return { kind: 'blocked', message: oneLine(error) };
    return { kind: 'failed', message: oneLine(error) };
  }
}

/**
 * How long the browser gets to answer a look that should be quick, such as the permission of a stored folder. Chrome
 * was seen to leave `queryPermission` of a stored handle unanswered for good (2026-11-19); without a limit a page that
 * waits for it stays on "starting" with nothing to click.
 */
export const LOOK_WAIT_MS = 2000;

/**
 * `promise`, or `fallback` once `ms` have passed without an answer. A promise that rejects in time still rejects. The
 * timer is cleared when the answer comes first.
 */
export function within(promise, ms, fallback) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => resolve(fallback), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

/**
 * The permission a stored handle has now for `mode`; `gone` when the folder does not exist any more. A browser that
 * does not answer within `waitMs` is taken to want to be asked (`prompt`): a click then asks with `grantAccess`, which
 * has no limit (it waits for the person's answer in the browser's dialog).
 */
export function permissionOf(handle, mode, waitMs = LOOK_WAIT_MS) {
  const asked = (async () => {
    try {
      return await handle.queryPermission({ mode });
    } catch (error) {
      return nameOf(error) === 'NotFoundError' ? 'gone' : 'denied';
    }
  })();
  return within(asked, waitMs, 'prompt');
}

/** Ask for the permission again. Must run inside a click (the first thing the click handler does). */
export async function grantAccess(handle, mode) {
  try {
    return await handle.requestPermission({ mode });
  } catch (error) {
    return nameOf(error) === 'NotFoundError' ? 'gone' : 'denied';
  }
}

// -- the folders used before ------------------------------------------------------------------------------------

const RECENT_KEY = 'recent';

/** Whether a stored value is a directory handle this module can use. */
function isHandle(value) {
  const h = value;
  return (
    typeof h === 'object' &&
    h !== null &&
    h.kind === 'directory' &&
    typeof h.values === 'function' &&
    typeof h.queryPermission === 'function' &&
    typeof h.requestPermission === 'function'
  );
}

/** A name for the list: what the folder is called, without control characters, cut short. */
export function displayName(name) {
  // eslint-disable-next-line no-control-regex
  return (typeof name === 'string' ? name : '').replace(/[\u0000-\u001f\u007f-\u009f]+/g, ' ').trim().slice(0, 200);
}

export class RecentFolders {
  kv;
  max;
  newId;

  constructor(kv, opts = {}) {
    this.kv = kv;
    this.max = opts.max ?? MAX_RECENT;
    this.newId = opts.newId ?? (() => crypto.randomUUID());
  }

  /** The folders, the one used last first. What is stored is checked: an entry that is not a folder is dropped. */
  async list() {
    let raw;
    try {
      raw = await this.kv.get(RECENT_KEY);
    } catch {
      return [];
    }
    if (!Array.isArray(raw)) return [];
    const out = [];
    const seen = new Set();
    for (const item of raw) {
      if (typeof item !== 'object' || item === null) continue;
      const { id, handle, lastUsed } = item;
      if (typeof id !== 'string' || id === '' || seen.has(id) || !isHandle(handle)) continue;
      seen.add(id);
      out.push({
        id,
        name: displayName(handle.name),
        handle,
        lastUsed: typeof lastUsed === 'number' && Number.isFinite(lastUsed) ? lastUsed : 0,
      });
    }
    return out.sort((a, b) => b.lastUsed - a.lastUsed).slice(0, this.max);
  }

  /** Remember `handle` as used at `now`: the same folder as one in the list moves up, a new one is added. */
  async remember(handle, now) {
    const all = await this.list();
    let same;
    for (const entry of all) {
      if (await sameFolder(entry.handle, handle)) {
        same = entry;
        break;
      }
    }
    const kept = all.filter((entry) => entry !== same);
    const entry = { id: same?.id ?? this.newId(), name: displayName(handle.name), handle, lastUsed: now };
    await this.save([entry, ...kept].slice(0, this.max));
    return entry;
  }

  async forget(id) {
    await this.save((await this.list()).filter((entry) => entry.id !== id));
  }

  save(list) {
    return this.kv.put(RECENT_KEY, list.map(({ id, handle, lastUsed }) => ({ id, handle, lastUsed })));
  }
}

async function sameFolder(a, b) {
  if (a === b) return true;
  try {
    return typeof a.isSameEntry === 'function' ? await a.isSameEntry(b) : false;
  } catch {
    return false;
  }
}

// -- IndexedDB ---------------------------------------------------------------------------------------------------

/**
 * One object store, handles by key, in the database `name`: the app's own (the origin is shared by every local file).
 * A directory handle can be stored in IndexedDB, in no other place.
 */
export function idbKeyValue(factory, name, store = 'handles') {
  if (typeof name !== 'string' || name === '') throw new Error('idbKeyValue: the name of the database is missing');
  // One connection for the page; a failed open is tried again next time.
  let opened = null;
  const db = () => {
    opened ??= new Promise((resolve, reject) => {
      const request = factory.open(name, 1);
      request.onupgradeneeded = () => request.result.createObjectStore(store);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    }).catch((error) => {
      opened = null;
      throw error;
    });
    return opened;
  };
  const run = async (mode, make) => {
    const d = await db();
    return new Promise((resolve, reject) => {
      const tx = d.transaction(store, mode);
      const request = make(tx.objectStore(store));
      tx.oncomplete = () => resolve(request.result);
      tx.onerror = () => reject(tx.error);
      // An aborted transaction (quota) fires no error of its own.
      tx.onabort = () => reject(tx.error ?? new Error('transaction aborted'));
    });
  };
  return {
    get: (key) => run('readonly', (s) => s.get(key)),
    put: async (key, value) => {
      await run('readwrite', (s) => s.put(value, key));
    },
  };
}
