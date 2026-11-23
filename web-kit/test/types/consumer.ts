// The kit's hand-written declarations (src/*.d.ts) against a consumer, the way an app calls them (tasks-desktop's
// ui/web/appkit-web.types.ts is the model). types.test.mjs only compares the exported names; this file makes a
// signature that no longer fits its callers fail. Nothing here runs: `pnpm check` (from web-kit/) type-checks it with
// the TypeScript of ui-kit (tsconfig.types.json). The imports go through the package's own name, so the `exports` map
// is checked too.

import {
  assemble,
  checkPartIds,
  CSP_STRICT,
  CSP_WASM_WORKER,
  inlineScript,
  inlineStyle,
  packPart,
  policyProblems,
  readDist,
  verify,
  verifyShell,
  writePage,
  type Bytes,
  type PageInput,
} from 'appkit-web/pack';
import {
  idbWrites,
  spyOnIndexedDb,
  violations,
  watchPage,
  type IdbWrite,
  type PageLike,
  type Watch,
} from 'appkit-web/e2e';
import {
  displayName,
  grantAccess,
  idbKeyValue,
  MAX_RECENT,
  oneLine,
  permissionOf,
  pickerSupport,
  pickFolder,
  RecentFolders,
  type AccessMode,
  type DirHandle,
  type KeyValue,
  type PermissionState,
  type PickerWindow,
  type PickResult,
  type Recent,
  type Support,
  type WritableFileHandle,
  type WritableStreamLike,
} from 'appkit-web/folder';
import {
  folderOfFiles,
  inputFolderSupport,
  NoPathsError,
  pickFolderByInput,
  type InputDocument,
  type InputPick,
  type PickedFile,
  type PickedFolder,
} from 'appkit-web/folder-input';
import { folderCache, forgetFolderCache, noCache, type ValueStore } from 'appkit-web/folder-cache';
import { WATCH_MS, watchFolder, type Watcher, type WatchDeps } from 'appkit-web/watch';
import { MAX_BYTES, blocks, problems as refProblems, taskProblems } from 'appkit-web/code-ref';

// -- appkit-web/code-ref: the check of a code-ref/1 a page makes before it uses one ------------------------------------

const refReasons: string[] = refProblems('{"schema":"code-ref/1"}');
const refBlocks: string[] = blocks('```code-ref\n{}\n```');
const refTask: Array<[number, string[]]> = taskProblems('');
const refs = [MAX_BYTES, refReasons, refBlocks, refTask, refProblems(new Uint8Array(0))];

// -- appkit-web/pack, as an app's build script calls it ------------------------------------------------------------

const PART_IDS = ['worker', 'wasm', 'bundle', 'shim'] as const;
const bytes: Bytes = new Uint8Array([1, 2, 3]);
const parts: Record<string, Uint8Array> = Object.fromEntries(PART_IDS.map((id) => [id, bytes]));
const { css, js }: { css: string; js: string } = readDist('dist-web', 'index.web.html');
const input: PageInput = { css, js, partIds: PART_IDS, parts, csp: CSP_WASM_WORKER, title: 'app', comment: 'build' };
const html: string = assemble(input);
// A page without parts: partIds and parts may be left out.
assemble({ css: inlineStyle(css), js: inlineScript(js), csp: CSP_STRICT, title: 'a', comment: 'b' });
const problems: string[] = [
  ...verifyShell(html, { partIds: PART_IDS }),
  ...verifyShell(html),
  ...verify(html, { allowUrls: ['https://example.invalid/'] }),
  ...verify(html),
];
const loose: string[] = policyProblems(CSP_STRICT);
const ids: readonly string[] = checkPartIds(PART_IDS);
const packed: string = packPart(bytes);
const sha: string = writePage('dist-web', 'app.html', html);

// -- appkit-web/e2e, with a page shaped like Playwright's Page (the kit does not depend on Playwright) -------------

interface PlaywrightLikePage {
  on(event: 'request', listener: (request: { url(): string; method(): string; headers(): object }) => void): this;
  on(event: 'pageerror', listener: (error: Error) => void): this;
  on(event: 'console', listener: (message: { type(): string; text(): string; args(): unknown[] }) => void): this;
  on(event: 'close', listener: () => void): this;
  addInitScript(script: (() => void) | string, arg?: unknown): Promise<void>;
  evaluate<R>(fn: () => R | Promise<R>): Promise<R>;
}
declare const page: PlaywrightLikePage;
const like: PageLike = page;
async function watched(): Promise<[Watch, string[], IdbWrite[]]> {
  const watch: Watch = await watchPage(page, { pageUrl: 'file:///app.html' });
  await spyOnIndexedDb(page);
  const writes: IdbWrite[] = await idbWrites(page);
  const said: string[] = await violations(like);
  const all: string[] = [...watch.requests, ...watch.violations, ...watch.errors];
  const ops: IdbWrite['op'][] = ['put', 'add', 'clear', 'delete'];
  return [watch, [...said, ...all], writes.filter((w) => w.db !== '' && w.store !== '' && ops.includes(w.op))];
}

// -- appkit-web/folder, as a page picks and keeps its folder --------------------------------------------------------

declare const win: Window & PickerWindow;
const support: Support = pickerSupport(win);
const mode: AccessMode = 'readwrite';
async function folder(): Promise<[Recent[], PermissionState | 'gone', string]> {
  const picked: PickResult = await pickFolder(win, { id: 'app-export', mode });
  if (picked.kind !== 'picked') {
    return [[], 'gone', picked.kind === 'cancelled' ? 'cancelled' : picked.message];
  }
  const handle: DirHandle = picked.handle;
  const kv: KeyValue = idbKeyValue(indexedDB, 'app-web');
  const recent = new RecentFolders(kv, { max: MAX_RECENT, newId: () => 'id' });
  const kept: Recent = await recent.remember(handle, Date.now());
  await recent.forget(kept.id);
  const file: WritableFileHandle = await handle.getFileHandle('a.txt', { create: true });
  const stream: WritableStreamLike = await file.createWritable({ keepExistingData: false });
  await stream.write(bytes);
  await stream.close();
  for await (const entry of handle.values()) {
    if (entry.kind === 'file') await entry.getFile();
  }
  const state: PermissionState | 'gone' = (await permissionOf(handle, 'read')) ?? (await grantAccess(handle, mode));
  return [await recent.list(), state, displayName(handle.name) + oneLine(new Error('x'), 80) + oneLine('y')];
}

// -- appkit-web/folder-input, the folder of an `<input webkitdirectory>` --------------------------------------------

const doc: InputDocument = document;
const inputs: boolean = inputFolderSupport(document) && inputFolderSupport(undefined);
async function byInput(): Promise<PickedFolder | string> {
  const pick: InputPick = await pickFolderByInput(doc);
  switch (pick.kind) {
    case 'picked':
      return pick.folder;
    case 'failed':
      return pick.message;
    default:
      return pick.kind;
  }
}
declare const files: FileList;
const fromInput: PickedFolder = folderOfFiles(files);
const asPicked: ArrayLike<PickedFile> = files;
const counted: number = fromInput.count;
const noPaths: Error = new NoPathsError();

// -- appkit-web/folder-cache and appkit-web/watch -------------------------------------------------------------------

interface Snapshot {
  areas: string[];
}
declare const store: ValueStore<Snapshot>;
const cache: ValueStore<Snapshot> = folderCache(store, 'D:/notes', (v) => v.areas.length > 0);
const none: ValueStore<Snapshot> = noCache;
async function cached(): Promise<Snapshot | null> {
  await forgetFolderCache(store, 'D:/notes');
  await cache.save({ areas: [] });
  return (await cache.load()) ?? (await none.load());
}

const deps: WatchDeps = {
  signature: async () => 'sig',
  onChange: () => undefined,
  onError: (error) => void oneLine(error),
  intervalMs: WATCH_MS,
  setTimer: (fn, ms) => setTimeout(fn, ms),
  clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  onFocus: (listener) => {
    window.addEventListener('focus', listener);
    return () => window.removeEventListener('focus', listener);
  },
};
const watcher: Watcher = watchFolder('sig', deps);
async function look(): Promise<boolean> {
  const changed: boolean = await watcher.check();
  watcher.accept('sig2');
  watcher.stop();
  return changed;
}

export const checked = [
  problems,
  loose,
  ids,
  packed,
  sha,
  watched,
  support,
  folder,
  inputs,
  byInput,
  asPicked,
  counted,
  noPaths,
  cached,
  look,
  refs,
] as const;
