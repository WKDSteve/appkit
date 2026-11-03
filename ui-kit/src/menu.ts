// The model of the menu bar and the logic that needs no DOM: how the items fall into sections, which menu an
// Alt+letter opens, which item a typed letter goes to, and how a label shows its mnemonic.

/**
 * An item that can be chosen may carry a `hint`: what it does or shows, in the hint popup (`tooltip.ts`) on hover and
 * focus of the item. A disabled button gets no pointer events from the browser, so its hint shows on focus only.
 *
 * An action or radio item that shows something may carry `onopennew`: the way a link opens in a new tab, a middle
 * click, Ctrl+click (Cmd+click) or Ctrl+Enter (Cmd+Enter) on it calls that instead of `onselect` (the menu bar only).
 * One that also has `href` (a place of the page, `route.ts`) is a link: the browser's own ways to open a link (its
 * context menu, Shift+click) open that address, and a plain click still calls `onselect` in the page.
 */
export type MenuItem =
  | {
      kind: 'action';
      label: string;
      onselect: () => void;
      onopennew?: () => void;
      href?: string;
      disabled?: boolean;
      shortcut?: string;
      hint?: string;
    }
  | {
      kind: 'radio';
      label: string;
      checked: boolean;
      onselect: () => void;
      onopennew?: () => void;
      href?: string;
      disabled?: boolean;
      shortcut?: string;
      hint?: string;
    }
  | { kind: 'check'; label: string; checked: boolean; onselect: () => void; shortcut?: string; hint?: string }
  | { kind: 'info'; label: string }
  | { kind: 'section'; label: string }
  | { kind: 'separator' };

export interface Menu {
  id: string;
  label: string;
  /** One letter of the label: Alt+letter opens the menu, and the letter is underlined in the bar. */
  mnemonic?: string;
  /** What the menu is for: shown by the hint popup (`tooltip.ts`) on hover and focus of the menu in the bar. */
  hint?: string;
  items: MenuItem[];
}

export type MenuBlock = { kind: 'divider' } | { kind: 'group'; heading: string | null; items: MenuItem[] };

/**
 * The items as blocks. A section starts a group with its heading and a divider before it; a separator is a divider
 * of its own. A section without items is dropped, and no divider stands first, last or next to another.
 */
export function toBlocks(items: readonly MenuItem[]): MenuBlock[] {
  const raw: MenuBlock[] = [];
  for (const item of items) {
    if (item.kind === 'separator') {
      raw.push({ kind: 'divider' });
    } else if (item.kind === 'section') {
      raw.push({ kind: 'divider' }, { kind: 'group', heading: item.label, items: [] });
    } else {
      const last = raw.at(-1);
      if (last && last.kind === 'group') last.items.push(item);
      else raw.push({ kind: 'group', heading: null, items: [item] });
    }
  }
  const out: MenuBlock[] = [];
  for (const block of raw) {
    if (block.kind === 'group' && block.items.length === 0) continue;
    if (block.kind === 'divider' && (out.length === 0 || out.at(-1)?.kind === 'divider')) continue;
    out.push(block);
  }
  while (out.at(-1)?.kind === 'divider') out.pop();
  return out;
}

/** The menu that Alt+`key` opens: the first whose mnemonic is that letter (case does not matter). */
export function menuForMnemonic(menus: readonly Menu[], key: string): Menu | undefined {
  if ([...key].length !== 1) return undefined;
  const k = key.toLocaleLowerCase();
  return menus.find((m) => m.mnemonic !== undefined && m.mnemonic.toLocaleLowerCase() === k);
}

/**
 * The letter of a physical key ('KeyP' is 'p'), for a layout where Alt+letter writes another character (Option on a
 * Mac): `event.key` is then that character, `event.code` still names the key.
 */
export function letterOfCode(code: string): string | undefined {
  const m = /^Key([A-Z])$/.exec(code);
  return m?.[1]?.toLowerCase();
}

/** A label cut at its mnemonic: the letter is marked, the rest stays. No mnemonic, or none in the label: no mark. */
export function splitMnemonic(
  label: string,
  mnemonic: string | undefined,
): { before: string; mark: string; after: string } {
  // Found letter by letter in the label itself: lower-casing a whole label can change its length ('İ' becomes two
  // code units), and an index from the changed text would cut the original in the wrong place.
  let at = -1;
  if (mnemonic) {
    const want = mnemonic.toLocaleLowerCase();
    for (let i = 0; i < label.length; ) {
      const ch = String.fromCodePoint(label.codePointAt(i) ?? 0);
      if (ch.toLocaleLowerCase() === want) {
        at = i;
        break;
      }
      i += ch.length;
    }
  }
  if (!mnemonic || at < 0) return { before: label, mark: '', after: '' };
  return {
    before: label.slice(0, at),
    mark: label.slice(at, at + mnemonic.length),
    after: label.slice(at + mnemonic.length),
  };
}

const KEY_NAMES: Record<string, string> = {
  ctrl: 'Control',
  control: 'Control',
  cmd: 'Meta',
  command: 'Meta',
  '⌘': 'Meta',
  meta: 'Meta',
  win: 'Meta',
  option: 'Alt',
  opt: 'Alt',
  alt: 'Alt',
  shift: 'Shift',
  esc: 'Escape',
  del: 'Delete',
};

/**
 * A shortcut as written for people ('Ctrl+Shift+N') in the form of `aria-keyshortcuts` ('Control+Shift+N'): the
 * names of the modifier keys are the ones the specification knows; a single letter stays as it is.
 */
export function ariaShortcut(shortcut: string): string {
  return shortcut
    .split('+')
    .map((part) => {
      const p = part.trim();
      return KEY_NAMES[p.toLocaleLowerCase()] ?? p;
    })
    .join('+');
}

/** How long a typed letter counts as part of the same word (milliseconds). */
export const TYPE_AHEAD_MS = 800;

/**
 * Where a typed word leads in a list of labels. One letter (or the same letter again and again) goes on from the
 * item after `from` to the next one that starts with it; a longer word looks from `from` on, so that it can stay on
 * the item it already names. -1 when nothing fits.
 */
export function typeAheadIndex(labels: readonly string[], from: number, typed: string): number {
  if (typed === '' || labels.length === 0) return -1;
  const word = typed.toLocaleLowerCase();
  const same = [...word].every((c) => c === word[0]);
  const prefix = same ? (word[0] ?? '') : word;
  const start = same ? from + 1 : Math.max(from, 0);
  for (let n = 0; n < labels.length; n++) {
    const i = (start + n) % labels.length;
    if (labels[i]?.toLocaleLowerCase().startsWith(prefix)) return i;
  }
  return -1;
}
