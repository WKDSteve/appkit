# appkit-web

The parts of an app's web variant that know no app. A web variant is one HTML file opened from disk (`file://`): no
server, no request of any kind, the data a folder the person picks. tasks-web was the first; testing-web is the next.
The Lua engine in the browser (the worker, wasmoon, the routes of the hub answered in the page) is not part of the kit:
it stays with the app that has one.

Plain JavaScript with its types beside it (`<module>.d.ts`): Node runs the packer from an app's `node_modules`, where it
strips no types, and an app's bundler takes the browser modules as they are. No dependencies.

| What | Module |
|---|---|
| Packing the page: `assemble({ css, js, partIds, parts, csp, title, comment })` writes one file with the script and the style inline, each part (`partIds`, any number, also none: a worker, a WASM module, data) gzip-compressed as base64 in a `<script id="<part>" type="application/octet-stream">`, and the policy as a meta tag. `verifyShell(html, { partIds })` checks the page without the contents of its scripts and styles: no element with a `src`, `href`, `data`, `action`, ... (any loading attribute), no URL, no refresh, a policy that passes `policyProblems(csp)`, every named part there once, one inline module and no other script (each script tag compared as a whole). `policyProblems(csp)` refuses a policy that lets the page load or reach anything: `default-src`, `connect-src`, `base-uri` and `form-action` must each be `'none'` alone (in CSP a `'none'` next to another source is ignored), no directive may name a source other than `'unsafe-inline'`, `'wasm-unsafe-eval'`, `data:` and `blob:` (no address, no wildcard, no `'unsafe-eval'`, no `data:` script), and no directive outside the fetch directives (no `report-uri`); `assemble` throws on such a policy. `verify(html, { allowUrls })` is the same check for a page whose scripts are part of it (a probe). Two policies: `CSP_WASM_WORKER` (the one tasks-web ships: `'wasm-unsafe-eval'`, `worker-src blob:`) and `CSP_STRICT` for a page without WASM and worker (neither of the two; a worker falls back to `script-src`, which allows no `blob:`, so none can start). Both have `default-src 'none'`, `connect-src 'none'`, `base-uri 'none'`, `form-action 'none'`. `readDist(dist, index)` finds the one script and style of a `vite build`; `writePage(out, name, html)` writes the file and its `SHA256SUMS` | `pack.js` |
| The folder the page works in: `pickerSupport`, `pickFolder(win, { id, mode })` (the picker of the File System Access API, under the app's own id; cancelled, blocked and failed told apart), `permissionOf(handle, mode, waitMs)` and `grantAccess(handle, mode)` (the permission the browser asks again after a restart, `gone` for a folder that is not there any more; a browser that leaves `queryPermission` unanswered for `LOOK_WAIT_MS` is taken as `prompt`, `within(promise, ms, fallback)` is that limit), `RecentFolders` (the folders used before, newest first, one entry per folder, a stored list that is not trusted) and `idbKeyValue(factory, name, store)`, the IndexedDB they are kept in. Every local HTML file shares the origin `file://`, and with it IndexedDB and the stored handles with their permission: each app keeps its handles in a database of its own `name` (tasks-web: `tasks-web`, testing-web: `testing-web`), never in another app's. The handle types (`DirHandleLike`, `WritableDirHandle`, ...) are the parts of the API a reader and a writer use | `folder.js` |
| A folder chosen through `<input webkitdirectory>`, for the browsers without the picker (Firefox, Safari): `inputFolderSupport`, `pickFolderByInput(document)` and `folderOfFiles(files)`, the flat list of files made a read-only folder a reader walks like any other. No handle: nothing is written, kept or looked at again | `folder-input.js` |
| The look at the folder: `watchFolder(known, deps)` asks for the signature of the folder (the app's: sizes and times) when the window gets the focus back and every 30 seconds, one look at a time, and tells the app when it differs | `watch.js` |
| The cache of a page that works on any folder: `folderCache(store, folder, releases)` keeps one value named by its folder and gives it back only to that folder and only while `releases(value)` holds (asked at every read and write: what is not released now is neither written nor given back, and goes); a folder the browser could not remember (`null`) keeps nothing. `forgetFolderCache` drops it when the person forgets the folder. The store is the app's (`indexedDbStore` of appkit-ui/cache, in a database of the app's own name) | `folder-cache.js` |
| The browser tests (Playwright; the page comes in by argument, the kit does not depend on Playwright): `watchPage(page, { pageUrl })` records every request that leaves the page (the page itself, `blob:` and `data:` are not the network), every error of the page and, through `violations(page)`, every violation of its policy; `spyOnIndexedDb(page)` and `idbWrites(page)` record every write to IndexedDB with its database, so a test can show that another app's database stayed untouched | `e2e.js` |
| The Node side of the browser tests, for the Playwright setup of an app: `removeTree(path, { tries, pauseMs })` (a folder a process still holds is retried, then an error, never a silent leftover), `stopChild(child)` (ends it and waits), `freePort()`, and for the hub the tests run against `pairingLink(config, hub)` and `sessionToken(config, hub)`, with `hub = { exe, env(config), origin }`, the three things that differ between apps | `e2e-node.js` |
| The check of a `code-ref/1` before a page uses one: `problems(bytes or text)` names every reason to refuse it (never throws), `blocks(markdown)` finds the fenced `code-ref` blocks of a task body, `taskProblems(markdown)` checks each; `leavesRepo(path)` is the one path rule. A port of `tools/check-code-ref.py`, tested against the goldens of `schemas/code-ref/1` | `code-ref.js` |

## Using it

An app depends on the folder of a tag of this repository, the same tag as `appkit-ui`:

```
pnpm add "git+https://github.com/WKDSteve/appkit.git#vX.Y.Z&path:web-kit"
```

pnpm writes that as `"appkit-web": "github:WKDSteve/appkit#vX.Y.Z&path:web-kit"`. The app keeps what is its own: the
parts and the policy of its page, its title, the id of its picker, the name of its database. tasks-desktop's
`ui/web/pack.mjs` and `ui/src/web/folder.ts` show the wiring:

```js
import { assemble, CSP_STRICT, verifyShell, writePage } from 'appkit-web/pack';

const html = assemble({ css, js, csp: CSP_STRICT, title: 'testing', comment: `testing-desktop web variant, build ${build}` });
const problems = verifyShell(html); // no part: none is expected
```

```ts
import { idbKeyValue, pickFolder, RecentFolders } from 'appkit-web/folder';

const recent = new RecentFolders(idbKeyValue(window.indexedDB, 'testing-web'));
const picked = await pickFolder(window, { id: 'testing-web-export', mode: 'read' });
```

To work on the kit and an app at once, point the app at the checkout for the time being
(`pnpm add link:../../appkit/web-kit`) and set the tag back before the commit.

## Checks

```
cd web-kit && pnpm test
cd web-kit && pnpm check      # needs `pnpm -C ../ui-kit install --frozen-lockfile` once
```

The tests are Node's own (`node --test`), with fakes for the window, the folders, IndexedDB (one fake origin with
databases by name) and the Playwright page. `test/types.test.mjs` checks that the declarations name what each module
exports. `pnpm check` type-checks the declarations against a consumer, `test/types/consumer.ts` (every module called
the way an app calls it, imported through the package's own `exports`, and a page shaped like Playwright's `Page` as
the kit's `PageLike`), with the TypeScript of ui-kit (`tsconfig.types.json`), so the kit itself installs nothing; CI
runs both in the job `web-kit`. The apps check their own calls too: tasks-desktop type-checks the calls of `pack` and
`e2e` it makes, and Playwright's real `Page` as `PageLike`, in `ui/web/appkit-web.types.ts` (`pnpm check`, through
`ui/tsconfig.kit.json`).
