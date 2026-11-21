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
  | { kind: 'separator' }
  | SubmenuItem;

/**
 * An item that opens a list of its own (one level: a submenu inside it is shown flat, `subItems`). Beside the menu
 * where there is room, folded open inside it in a narrow window. When one of its `radio` items is checked, the item
 * shows the dot and its name says which (`submenuCurrent`).
 */
export interface SubmenuItem {
  kind: 'submenu';
  label: string;
  items: MenuItem[];
  disabled?: boolean;
  hint?: string;
}

/**
 * The items of a submenu, one level deep: a submenu inside it gives way to a section with its label and its items
 * (flat, at any depth), so nothing is lost and no third level opens.
 */
export function subItems(item: SubmenuItem): PlainItem[] {
  return flatItems(item.items);
}

/** An item that opens no list of its own. */
export type PlainItem = Exclude<MenuItem, SubmenuItem>;

/**
 * The items with every submenu laid open: a section with its label, then its items (at any depth). For a list that
 * opens no submenu (the context menu), and for the level below a submenu.
 */
export function flatItems(items: readonly MenuItem[]): PlainItem[] {
  const out: PlainItem[] = [];
  const add = (list: readonly MenuItem[]): void => {
    for (const it of list) {
      if (it.kind === 'submenu') {
        out.push({ kind: 'section', label: it.label });
        add(it.items);
      } else out.push(it);
    }
  };
  add(items);
  return out;
}

/** The label of the checked `radio` item of a submenu (the first, at any depth), or null. */
export function submenuCurrent(item: SubmenuItem): string | null {
  for (const it of subItems(item)) if (it.kind === 'radio' && it.checked) return it.label;
  return null;
}

/** Whether a submenu has something that can be chosen; one without is shown as disabled. */
export function submenuUsable(item: SubmenuItem): boolean {
  if (item.disabled) return false;
  return subItems(item).some(
    (it) =>
      it.kind === 'check' || ((it.kind === 'action' || it.kind === 'radio') && !it.disabled),
  );
}

/** Below this width of the window (pixels) a submenu folds open inside its menu instead of beside it. */
export const SUBMENU_NARROW = 410;

/** How long the pointer rests on a submenu item before it opens (milliseconds). */
export const SUBMENU_OPEN_MS = 150;

/** How long a submenu stays open while the pointer crosses a neighbour on its way to it (milliseconds). */
export const SUBMENU_GRACE_MS = 300;

interface Box {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/**
 * Where a submenu `size` big goes beside its item `parent` in a window `view` big: to the right where it fits, else
 * to the left, else to the side with more room, kept inside the window. Its first item lines up with the parent
 * (`lift`: the padding and border above it).
 */
export function flyoutPlace(
  parent: Box,
  size: { width: number; height: number },
  view: { width: number; height: number },
  lift = 0,
  margin = POPUP_MARGIN,
): { left: number; top: number; side: 'right' | 'left' } {
  const roomRight = view.width - margin - parent.right;
  const roomLeft = parent.left - margin;
  const side = size.width <= roomRight || (size.width > roomLeft && roomRight >= roomLeft) ? 'right' : 'left';
  let left = side === 'right' ? parent.right : parent.left - size.width;
  left = Math.max(margin, Math.min(left, view.width - margin - size.width));
  let top = parent.top - lift;
  top = Math.max(margin, Math.min(top, view.height - margin - size.height));
  return { left, top, side };
}

/**
 * Whether the pointer at `point`, coming from `origin` (where it left the submenu's item), is on its way to the
 * submenu `target` on `side`: inside the triangle from `origin` to the near edge of the submenu. Then crossing a
 * neighbouring item does not close the submenu at once. The tip stays where the pointer left (a tip that follows the
 * pointer would close the submenu on the first step off a straight line), moved `slack` pixels away from the submenu
 * so that the pointer's own place is inside rather than on the edge.
 */
export function inCorridor(
  origin: { x: number; y: number },
  point: { x: number; y: number },
  target: Box,
  side: 'right' | 'left',
  slack = 5,
): boolean {
  const x = side === 'right' ? target.left : target.right;
  const a = { x: origin.x + (side === 'right' ? -slack : slack), y: origin.y };
  const b = { x, y: target.top };
  const c = { x, y: target.bottom };
  const cross = (p: { x: number; y: number }, q: { x: number; y: number }, r: { x: number; y: number }): number =>
    (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
  const d1 = cross(a, b, point);
  const d2 = cross(b, c, point);
  const d3 = cross(c, a, point);
  const neg = d1 < 0 || d2 < 0 || d3 < 0;
  const pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

export interface Menu {
  id: string;
  label: string;
  /** One letter of the label: Alt+letter opens the menu, and the letter is underlined in the bar. */
  mnemonic?: string;
  /** What the menu is for: shown by the hint popup (`tooltip.ts`) on hover and focus of the menu in the bar. */
  hint?: string;
  /**
   * Called each time the menu opens (click, key, mnemonic, the pointer moving over from an open neighbour), before
   * its items are drawn: the place to read again what the items show (a list of folders, a state of access). A menu
   * that is open from the start (`initialOpen` of the menu bar) is told once when the bar is mounted, after its items
   * were drawn the first time; what the hook changes then is drawn at once.
   */
  onopen?: () => void;
  items: MenuItem[];
}

export type MenuBlock<T extends MenuItem = MenuItem> =
  | { kind: 'divider' }
  | { kind: 'group'; heading: string | null; items: T[] };

/**
 * The items as blocks. A section starts a group with its heading and a divider before it; a separator is a divider
 * of its own. A section without items is dropped, and no divider stands first, last or next to another.
 */
export function toBlocks<T extends MenuItem>(items: readonly T[]): MenuBlock<T>[] {
  const raw: MenuBlock<T>[] = [];
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
  const out: MenuBlock<T>[] = [];
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

/**
 * Whether a popup can be anchored to its trigger by the platform: the Popover API (top layer) and CSS anchor
 * positioning together. Where either is missing (Chrome before 125, an older Safari or Firefox, jsdom) the popup stays
 * where the page puts it, inside the window by `popupShift`. Feature detection, never the user agent.
 */
export function canAnchorPopover(env: {
  showPopover?: unknown;
  supports?: (condition: string) => boolean;
}): boolean {
  if (typeof env.showPopover !== 'function' || typeof env.supports !== 'function') return false;
  try {
    return env.supports('position-area: block-end') && env.supports('anchor-name: --a');
  } catch {
    return false;
  }
}

/** The anchor name of a trigger: a dashed ident made of what an id can hold. */
export function anchorName(prefix: string, id: string): string {
  return `--${prefix}-${id}`.replace(/[^A-Za-z0-9_-]/g, '_');
}

/** How far a popup keeps from the edges of the window, in pixels. */
export const POPUP_MARGIN = 8;

/**
 * How far to move a popup sideways (negative: to the left) so that it lies inside a window `viewWidth` wide, given
 * where it lies now. A popup wider than the window keeps its left edge at the margin.
 */
export function popupShift(left: number, right: number, viewWidth: number, margin = POPUP_MARGIN): number {
  let shift = 0;
  if (right > viewWidth - margin) shift = viewWidth - margin - right;
  if (left + shift < margin) shift = margin - left;
  return shift;
}
