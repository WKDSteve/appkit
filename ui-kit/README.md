# appkit-ui

The parts of the browser page that every app of appkit shares, as Svelte 5 and TypeScript source: an app's
bundler (Vite with the Svelte plugin) compiles them, there is no build here.

| What | File |
|---|---|
| The design tokens in ten groups: surfaces, text, lines and accent (`--line`, `--accent-soft`, `--focus`), meaning colours, depth and marks (`--shadow`, `--shadow-sm`, `--backdrop`, `--mark`), shape, type (`--font-size`, `--line-height`), the palette of an app (`--accent-hover`, `--secondary`, `--tertiary`, `--chart-1` to `--chart-6`, `--accent-50` to `--accent-900`; see "The palette of an app"), the theme (`light-dark()` follows the system, `data-theme` on the root overrides it), the base rules, and the check `tools/check-color-literals.py` (which every app runs over its own page too): no hex colour or colour function where the CSS of a page states a colour, outside the tokens; scripts, `.ts` and `.svg` files and named colours are not looked at. `--border` is an alias of `--line` while the pages move | `tokens.css` |
| The base rules that come with them: box model, page, focus ring, headings, the hint popup, the info button, `.sr-only`, reduced motion | `base.css` |
| The menu bar of an application (a row of menus with the keys of the ARIA pattern, sections with a heading and a line, shortcuts on the right, Alt+letter mnemonics, type-ahead) | `MenuBar.svelte`, `menu.ts` |
| The command palette (Ctrl+K): the box, the list and the keys in `Palette.svelte`; the loose matching, the frecency (what was chosen before comes first) and the list of actions and entries (tasks, rules) as plain functions in `palette.ts`. The page builds the list, runs the choice and keeps the frecency, so the component knows neither `App` nor a data contract | `Palette.svelte`, `palette.ts` |
| The conflict center: the changes the engine refused because the file moved on, kept in one place; for each what was wanted next to what is there now, and the actions the app offers (try again, look, let go). The person chooses how it is shown, every conflict in a list (the default) or one at a time with a count of the rest; the app stores that choice (`parseConflictMode`). The kit writes and merges nothing | `ConflictCenter.svelte`, `conflicts.ts` |
| The one-line message of a page with the way back (an error is announced as an alert, the rest in the status region); the page keeps the message and its timer | `Toast.svelte`, `toast.ts` |
| A choice of any number of values, with a search when the list is long (it finds an option by its label, its value or its `title`, so an alias in `title` is found) and an optional `hint` on its button (described to a screen reader through the hint popup, and not shown while the choice is open). An optional `note` says at the top of the popup how the chosen values combine; an optional `match` adds an "any of them / all of them" switch for a field a record can hold several values of. While the popup is open the button keeps the width it had when it opened, so a label that names the choice ("Areas: 2") does not move the row and the popup under the pointer | `MultiSelect.svelte` |
| The "what does this mean" button beside a label | `Info.svelte` |
| The context menu (right click, the context key, Shift+F10): the menu belongs to the target under the pointer, not to the page. An element declares what it is (`use:contextTarget={{ kind: 'task', id }}`), an app registers providers per kind (`registry.provide('tile', (target, env) => [...])`), and the menu is what the providers of every target around the pointer say, the innermost first (tile, view, tab): an entry with an `id` of an inner provider wins over the same `id` further out, `{ kind: 'hide', id }` takes an outer entry away, the whole selection is handed on when the target is part of it. Entries are the `MenuItem`s of the menu bar. The browser's own menu stays where nobody offers anything and in text fields; Up, Down, Home, End, Enter, Escape (the focus goes back), a click beside, a scroll that moves the target and a resize | `contextmenu.ts`, `ContextMenu.svelte` |
| A dialog that closes when the mouse is pressed and released on the backdrop beside it (`use:closeOnBackdrop` on a `<dialog>`; `closeable` can keep one open that holds a draft); the command palette and the documentation window use it | `backdrop.ts` |
| The name of the app at the left end of the bar: a button with a quiet hover effect (a line under it, the accent, a little more letter spacing, none of the movement for reduced motion) when the app gives it a click, plain text without | `Brand.svelte` |
| The documentation inside an app: `searchDocs` over the documents (every word, the title weighs more), `helpItems` for the Help menu (the section Documentation with the search and one entry per document, the section Project with the repository and the license; only https links), the window `DocsDialog.svelte` (search, list, the chosen document as Markdown) and `AboutLinks.svelte` (the repository with the mark of GitHub, the license). The app brings the documents, bundled with the page | `docs.ts`, `DocsDialog.svelte`, `AboutLinks.svelte` |
| A markdown reader that draws text as elements, never as an HTML string | `markdown.ts`, `Blocks.svelte`, `Inlines.svelte` |
| One explanation popup for every element with `data-hint` (hover, focus, Escape) | `tooltip.ts` |
| The hub client every app extends (`class Hub extends HubClient`): the session as a bearer token and nothing else (no cookie, no cache, no referrer), a late refusal of an old token ignored, the refusal as `ApiError`, the pairing code traded for a token, `hello`, `snapshot` with its tag, `rescan`, and `postJson`/`post` for the calls of the app. The app names where its routes live (`/api/tasks`) and adds its own calls | `client.ts` |
| What an app that keeps a copy in the browser shares: `cacheableClass` (only `public` and `internal`, fail closed) and an IndexedDB store behind `CacheStore` (every failure is a miss). What is kept, and how a copy is cut, is the app's | `cache.ts` |
| The check that the shell of every app is the same one: `collectShell` reads, in a browser, the styles the browser works out for the page, the menu bar, an open menu and the palette; `diffShell` compares them with `shell.golden.json` (light and dark), one line per property that differs. Styles, not pixels, so the fonts of a machine do not matter. An app runs it in its browser tests; the golden file is changed only on purpose, in the kit | `shell-check.ts`, `shell.golden.json` |
| What every page has the same way: `Preferences` (the language, the theme and whether single keys are shortcuts, read back from the profile and kept in it, a damaged or blocked profile giving the defaults) and `StreamSession` (the connection to the hub: the event stream read again after a break, a little later each time up to half a minute, until the page stops, the session is gone or the hub refuses it with a 401). What a page does with a message and with the loss of the session, and how it starts and trades a pairing code, is the app's | `session.svelte.ts` |
| A server-sent-event reader over `fetch` (a bearer header, a backoff) | `sse.ts` |
| The pairing code in the URL fragment, taken out of the address bar (`keepRoute`: the place of the page stays) | `pair.ts` |
| Back and Forward: each place is an entry of the tab's history, the state in `history.state`; with `address` the place is also in the fragment, so a reload, a copied address and a link in a new tab open it (`placeOf` at the start), a place typed into the address is gone to, and `record(..., { replace: true })` changes the place without a step for Back (a filter) | `nav.ts` |
| The place of a page as parameters of the fragment (`#view=board&task=a%2Fb`), never the path (the hub serves `/` alone, the web variant is a file): `readRoute`, `routeHash`, `routeHref` for a link's `href`. A menu item with `href` is a link (`<a>`): its context menu and Shift+click open it in a new tab, a plain click stays in the page | `route.ts`, `menu.ts` |
| The session for a new tab: a tab without a token asks the open tabs of the page on a `BroadcastChannel` (`askForSession`), one whose event stream is open answers (`shareSession`); no answer in 400 ms offers a pairing as usual, and an answer that comes later still counts (`onLate`). SECURITY.md says what limits it | `handoff.ts` |
| Characters that hide or reverse text, made visible | `text.ts` |
| The keyboard as data: `keymap.json` says which key is which command (Ctrl or Cmd and K for the palette, `j`/`k`, Enter, Escape, `/`, a chord that starts with `g`); `resolveKey` turns a key press into a command by name, without a DOM. An app adds the targets of its chord with `withChord` and acts on the command | `keymap.json`, `keymap.ts` |
| Filling the holes of a text, choosing a language, looking a key up, `ago`, `formatNumber`, `labelOf`/`hintOf` (the word and the explanation of a value, a new value shows as itself); the words belong to the app, except `kitTexts`, those of the kit's own components under the prefix `kit.` | `i18n.ts` |

## Using it

An app depends on the folder of a tag of this repository:

```
pnpm add "git+https://github.com/WKDSteve/appkit.git#vX.Y.Z&path:ui-kit"
```

`pnpm-lock.yaml` then pins the commit. In the page:

```ts
import 'appkit-ui/tokens.css';
import 'appkit-ui/base.css';
import MenuBar, { type Menu } from 'appkit-ui/MenuBar.svelte';
import { installTooltips, tooltip } from 'appkit-ui/tooltip';
```

### The palette of an app

The tokens are blue. An app takes the colours of its icon by loading its palette after `tokens.css` and before
`base.css`:

```ts
import 'appkit-ui/tokens.css';
import './palette.css';
import 'appkit-ui/base.css';
```

`palette.css` is generated, never written by hand: `node palettes.mjs install` (the tool of the app icons, kept
outside this repository) writes it into the repository of each app, and `node palettes.mjs check` names a copy that differs. It sets the
colours of the tokens (surfaces, text, `--line`, `--accent`, `--accent-text`, `--focus`, the meaning colours stay
those of the kit) and those of group 3b: `--accent-hover` (the fill of an accent button under the pointer),
`--secondary`, `--tertiary`, `--chart-1` to `--chart-6` and the scale `--accent-50` to `--accent-900`. Without a
palette these have the values of the blue of the kit. The check of an app names the palette as a second file that may
hold colours: `python tools/check-color-literals.py --root ui/src --tokens ui/src/palette.css`.

The kit has almost no words: `installTooltips` takes the name of its region from the app (`regionLabel`), the components
take every label as a prop, and `i18n.ts` only fills holes and picks a language.

A menu is a list of items; `{ kind: 'section', label }` starts a group with a heading (a line is drawn before it),
`{ kind: 'separator' }` is a line of its own, and an action, radio or check item may carry `shortcut: 'Ctrl+N'`. The
shortcut is shown on the right and announced (`aria-keyshortcuts`); the app binds the key itself. A menu may carry a `hint`, which the hint popup shows on its
trigger in the bar (not while the menu is open: the popup would lie over its first items). An action, radio or check item may carry a `hint` too: it shows on hover and focus of the item (a disabled item gets no pointer events from the browser, so its hint shows on focus only). A menu with
`mnemonic: 'p'` opens on Alt+P and shows the letter underlined. Inside an open menu the arrow keys, Home and End walk
the items, a letter goes to the next item that starts with it, Left and Right change the menu, Escape closes. An action
or radio item that shows something may carry `onopennew`: a middle click, Ctrl+click or Ctrl+Enter (Cmd on a Mac) calls
it instead of `onselect`, the way a link opens in a new tab; without it these act as a plain choice.
Alt+letter is left alone while a modal dialog is open or when another handler took the key; on a Mac, where Option+letter
writes another character, the physical key counts. Changed since the first version of the menu: `MenuItem` has a
member more (`section`), so an exhaustive `switch` over `kind` needs a case for it, and a separator at the start or the
end of a list, or two in a row, is no longer drawn.

A hint is the text of the app, from its dictionary. Set it as `data-hint` (optionally `data-hint-title`, and
`data-hint-delay` in milliseconds), or with the action, which sets exactly these:

```svelte
<button use:tooltip={t('save.hint')}>Save</button>
<span tabindex="0" use:tooltip={{ text: t('live.hint'), title: t('live.title'), delay: 600 }}>live</span>
```

The element must be reachable by keyboard (a button, a link, or `tabindex="0"`): the popup shows on hover after the
pause, at once on focus and at once on a tap (touch; a tap elsewhere takes it away). When the action updates the text
the open popup follows, and when the element goes away the popup goes with it. It stays while the pointer is on it, and goes with Escape, on scrolling and when the pointer
or the focus leaves. A press inside it keeps it open (its text can be selected and copied) until a press outside or
Escape. When it had to lie on a control it lets the pointer through, so the control under it is reached; and when the
pointer moves to another element with a hint, the popup of the one it left goes at once.

To work on the kit and an app at once, point the app at the checkout (`pnpm add link:../path/to/appkit/ui-kit`) and
put the tag back before committing.

## Checks

```
pnpm install --frozen-lockfile
pnpm check
pnpm test
```
