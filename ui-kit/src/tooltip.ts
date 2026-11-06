// One explanation popup for the whole page. Any element with `data-hint="text"` gets it: on hover (after a short
// delay) and on keyboard focus. It can be reached with the pointer (it stays while the pointer is on it) and goes
// away with Escape, on scrolling and when the pointer or the focus leaves (WCAG 1.4.13). A press inside it keeps it
// open (its text can be selected and copied) until a press outside or Escape. A popup that had to lie on a control
// lets the pointer through (pointer-events: none), so the control under it is reached and its own hint shows. The
// text is set as text, never as markup.

/** What an app tells the popup: the name of its region, in the app's language (the kit has no words of its own). */
export interface TooltipOptions {
  regionLabel: () => string;
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A side of its element the popup stands on. */
export type Side = 'below' | 'above' | 'right' | 'left';

/**
 * Where the popup goes: below the element, else above it, else to its right or left - the first place that fits in
 * the window and that `free` says covers no other control (a popup that stays while the pointer is on it would catch
 * the click meant for the control under it). When no place is free it goes below (above when there is no room below),
 * kept inside the window; the pointer goes through it then (pointer-events: none). With `only` it stays on that side (kept inside the window):
 * a popup that follows a scrolling element does not jump from side to side.
 *
 * Since v0.14.0 a popup that fits neither below nor above goes to the right or left when that fits (before, it was
 * pushed into the window over its element).
 */
export function placeTip(
  anchor: Box,
  tip: { width: number; height: number },
  view: { width: number; height: number },
  gap = 6,
  free: (box: Box) => boolean = () => true,
  only?: Side,
): { x: number; y: number; side: Side } {
  const clampX = (x: number) => Math.max(0, Math.min(x, Math.max(0, view.width - tip.width)));
  const clampY = (y: number) => Math.max(0, Math.min(y, Math.max(0, view.height - tip.height)));
  const below = { x: clampX(anchor.x), y: anchor.y + anchor.height + gap };
  const above = { x: clampX(anchor.x), y: anchor.y - gap - tip.height };
  const right = { x: anchor.x + anchor.width + gap, y: clampY(anchor.y) };
  const left = { x: anchor.x - gap - tip.width, y: clampY(anchor.y) };
  const fits = (p: { x: number; y: number }) =>
    p.x >= 0 && p.y >= 0 && p.x + tip.width <= view.width && p.y + tip.height <= view.height;
  const places = { below, above, right, left };
  if (only) {
    const p = places[only];
    return { x: clampX(p.x), y: clampY(p.y), side: only };
  }
  for (const side of ['below', 'above', 'right', 'left'] as const) {
    const p = places[side];
    if (fits(p) && free({ ...p, width: tip.width, height: tip.height })) return { ...p, side };
  }
  const up = below.y + tip.height > view.height && above.y >= 0;
  return { x: below.x, y: clampY(up ? above.y : below.y), side: up ? 'above' : 'below' };
}

/**
 * What a press can mean: the popup must not lie on one of these (of another element). Not every element with a
 * `tabindex`: a region that scrolls by the keyboard has one and is no control.
 */
export const CONTROL =
  'a[href], button, input, select, textarea, summary, label, [contenteditable]:not([contenteditable="false"]), ' +
  '[role="button"], [role="link"], [role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"], ' +
  '[role="checkbox"], [role="radio"], [role="switch"], [role="option"], [role="tab"], [role="combobox"], ' +
  '[role="slider"], [role="treeitem"]';

/** What a popup must not lie on: a control, or another element with a hint of its own. */
const KEEP_FREE = `${CONTROL}, [data-hint]`;

/**
 * Whether a control is the hint's own (the element itself, or one it holds: a label and its field): those may be
 * covered. A control that holds the element is not its own - a hint on a tag inside a card must not lie on the card.
 */
export function ownControl(control: Node, el: Node): boolean {
  return control === el || el.contains(control);
}

/**
 * Whether `box` lies on a control: one of `controls` whose rectangle meets it and that is really there in the middle
 * of the overlap (`topControlAt` names the control the topmost thing at a point belongs to; one hidden under another
 * layer is not pressed). Every control is looked at, a small one too.
 */
export function liesOnControl<C>(
  box: Box,
  controls: readonly { rect: Box; control: C }[],
  topControlAt: (x: number, y: number) => C | null,
): boolean {
  for (const { rect, control } of controls) {
    const x0 = Math.max(box.x, rect.x);
    const y0 = Math.max(box.y, rect.y);
    const x1 = Math.min(box.x + box.width, rect.x + rect.width);
    const y1 = Math.min(box.y + box.height, rect.y + rect.height);
    if (x1 <= x0 || y1 <= y0) continue;
    if (topControlAt((x0 + x1) / 2, (y0 + y1) / 2) === control) return true;
  }
  return false;
}

/** The pause before a popup shows on hover, and the one before it goes when the pointer left. */
export const SHOW_DELAY_MS = 300;
export const HIDE_DELAY_MS = 150;

export const TIP_ID = 'hint-tip';

/** The longest pause an element may ask for with `data-hint-delay`. */
export const MAX_DELAY_MS = 5000;

/** The pause of one element: its `data-hint-delay` when that is a number of milliseconds, else the default. */
export function hintDelay(raw: string | undefined): number {
  if (raw === undefined || !/^\d+$/.test(raw.trim())) return SHOW_DELAY_MS;
  return Math.min(Number(raw), MAX_DELAY_MS);
}

/** What the `tooltip` action takes: a text, or a text with a title and a pause of its own (milliseconds). */
export type TooltipParams = string | { text: string; title?: string; delay?: number };

/** The `data-hint*` attributes of the params; an empty text gives none, so the element has no popup. */
export function hintAttributes(params: TooltipParams): Record<string, string> {
  const p = typeof params === 'string' ? { text: params } : params;
  if (p.text === '') return {};
  const attrs: Record<string, string> = { 'data-hint': p.text };
  if (p.title) attrs['data-hint-title'] = p.title;
  if (p.delay !== undefined && Number.isFinite(p.delay) && p.delay >= 0) {
    attrs['data-hint-delay'] = String(Math.floor(p.delay));
  }
  return attrs;
}

const HINT_ATTRIBUTES = ['data-hint', 'data-hint-title', 'data-hint-delay'];

// The action tells the popups that are installed when an element changed its hint or went away: a popup open for
// it would otherwise show the old text, or stay on the screen for an element that is no longer there (no focusout
// comes for a removed element).
type Change = (el: HTMLElement, gone: boolean) => void;
const watchers = new Set<Change>();
const notify = (el: HTMLElement, gone: boolean): void => watchers.forEach((w) => w(el, gone));

/**
 * Svelte action: `<button use:tooltip={t('save.hint')}>` or `use:tooltip={{ text, title, delay }}`. It only sets the
 * attributes the popup of `installTooltips` looks for, so the element must be reachable by keyboard (a button, a
 * link, or `tabindex="0"`) for the popup to show on focus.
 */
export function tooltip(node: HTMLElement, params: TooltipParams): { update(next: TooltipParams): void; destroy(): void } {
  const apply = (p: TooltipParams): void => {
    for (const name of HINT_ATTRIBUTES) node.removeAttribute(name);
    for (const [name, value] of Object.entries(hintAttributes(p))) node.setAttribute(name, value);
  };
  apply(params);
  return {
    update: (next) => {
      apply(next);
      notify(node, false);
    },
    destroy: () => {
      for (const name of HINT_ATTRIBUTES) node.removeAttribute(name);
      notify(node, true);
    },
  };
}

/** Join an id into a space separated list (`aria-describedby`) and take it out again. */
export function withToken(list: string | null, token: string): string {
  const tokens = (list ?? '').split(/\s+/).filter(Boolean);
  return (tokens.includes(token) ? tokens : [...tokens, token]).join(' ');
}
export function withoutToken(list: string | null, token: string): string {
  return (list ?? '')
    .split(/\s+/)
    .filter((t) => t !== '' && t !== token)
    .join(' ');
}

/** Install the popup on a document. Returns a function that takes it away again. */
export function installTooltips(
  doc: Document,
  win: Window & typeof globalThis,
  options: TooltipOptions,
): () => void {
  // The popup lives in a region of its own that is there only while a hint is open: content outside every
  // landmark is a finding of its own, and an always present empty region would be noise for a screen reader.
  const host = doc.createElement('div');
  host.className = 'hint-host';
  host.setAttribute('role', 'region');
  host.hidden = true;
  const tip = doc.createElement('div');
  tip.id = TIP_ID;
  tip.className = 'hint-tip';
  tip.setAttribute('role', 'tooltip');
  // Closed from the start: `tip.hidden` is what the Escape key and the hide timers ask, and a popup that never
  // opened must not take the Escape of a dialog or a menu.
  tip.hidden = true;
  host.appendChild(tip);
  doc.body.appendChild(host);
  // The name of the region is the app's word (its dictionary): the kit has none, so the app must give one.
  const regionLabel = options.regionLabel;

  let current: HTMLElement | null = null;
  let describedBefore: string | null = null;
  let viaFocus = false;
  // The button of the mouse is down: the focus that comes with it is the press, not the keyboard.
  let pressing = false;
  // A press inside the popup keeps it: it goes with a press outside or Escape, not with the pointer or the focus.
  let pinned = false;
  let showTimer: number | undefined;
  let hideTimer: number | undefined;

  const hintOf = (target: EventTarget | null): HTMLElement | null =>
    target instanceof win.Element ? target.closest<HTMLElement>('[data-hint]') : null;

  // A timer that is replaced is stopped first: a lost timer would still fire and take the popup away (or the
  // pending show of the next element) under the pointer.
  function cancelShow(): void {
    win.clearTimeout(showTimer);
    showTimer = undefined;
  }
  function cancelHide(): void {
    win.clearTimeout(hideTimer);
    hideTimer = undefined;
  }
  function scheduleHide(): void {
    cancelHide();
    hideTimer = win.setTimeout(hide, HIDE_DELAY_MS);
  }

  /** The element describes itself (`Info.svelte` keeps the text in the page): the popup adds nothing to it. */
  const ownsDescription = (el: HTMLElement): boolean => el.hasAttribute('data-hint-own');

  function hide(): void {
    cancelShow();
    cancelHide();
    if (current && !ownsDescription(current)) {
      // Only what this popup added is taken away: a description the element had of its own stays.
      const rest = withoutToken(current.getAttribute('aria-describedby'), TIP_ID);
      if (rest === '' && describedBefore === null) current.removeAttribute('aria-describedby');
      else current.setAttribute('aria-describedby', rest);
    }
    current = null;
    viaFocus = false;
    pinned = false;
    tip.style.pointerEvents = '';
    tip.hidden = true;
    host.hidden = true;
    if (host.parentElement !== doc.body) doc.body.appendChild(host);
  }

  /**
   * No control under `box` but the element's own (it, and what it holds). A control that holds the element counts:
   * a hint on a tag inside a card must not lie on the card, whose press selects it. Another element with a hint counts
   * too: the pointer on its way there would rest on the popup and never show the next hint.
   */
  function freeFor(el: HTMLElement): (box: Box) => boolean {
    // A document without hit testing (a test without layout) has nothing to say.
    if (typeof doc.elementsFromPoint !== 'function') return () => true;
    const controls: { rect: Box; control: Element }[] = [];
    for (const control of doc.querySelectorAll(KEEP_FREE)) {
      if (ownControl(control, el) || host.contains(control)) continue;
      const r = control.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) {
        controls.push({ rect: { x: r.x, y: r.y, width: r.width, height: r.height }, control });
      }
    }
    const topControlAt = (x: number, y: number): Element | null =>
      doc
        .elementsFromPoint(x, y)
        .find((e) => !host.contains(e))
        ?.closest(KEEP_FREE) ?? null;
    return (box) => !liesOnControl(box, controls, topControlAt);
  }

  /** The side the popup was put on when it was shown: a scroll moves it with its element, on that side. */
  let side: Side | undefined;

  function place(el: HTMLElement, keepSide = false): void {
    const anchor = el.getBoundingClientRect();
    // Measured at the corner of the window: where it stood before (for another element) caps its width there.
    tip.style.left = '0px';
    tip.style.top = '0px';
    const size = tip.getBoundingClientRect();
    const free = keepSide && side ? () => true : freeFor(el);
    const at = placeTip(
      { x: anchor.x, y: anchor.y, width: anchor.width, height: anchor.height },
      { width: size.width, height: size.height },
      { width: win.innerWidth, height: win.innerHeight },
      6,
      free,
      keepSide ? side : undefined,
    );
    side = at.side;
    // No free place: the popup lies on a control. The pointer goes through it, so that the control is pressed and
    // its own hint shows (before, the popup stayed under the pointer and hid the next entry of a menu).
    if (!keepSide && !pinned) tip.style.pointerEvents = free({ x: at.x, y: at.y, width: size.width, height: size.height }) ? '' : 'none';
    tip.style.left = `${at.x}px`;
    tip.style.top = `${at.y}px`;
  }

  function show(el: HTMLElement, byFocus: boolean): void {
    const text = el.dataset.hint ?? '';
    if (text === '') return hide();
    if (current && current !== el) hide();
    // The same element again (hovered, then clicked and focused): what it described before is not read anew,
    // or the popup's own id would count as the element's.
    if (current !== el) describedBefore = el.getAttribute('aria-describedby');
    current = el;
    viaFocus = viaFocus || byFocus;
    const title = el.dataset.hintTitle ?? '';
    // A kept popup whose text did not change is left as it is: rebuilding it would drop the selection in it.
    if (pinned && current === el && tip.textContent === title + text) return;
    tip.replaceChildren();
    if (title !== '') {
      const heading = doc.createElement('strong');
      heading.className = 'hint-title';
      heading.textContent = title;
      tip.appendChild(heading);
    }
    tip.appendChild(doc.createTextNode(text));
    host.setAttribute('aria-label', regionLabel());
    // A modal dialog lies in the top layer and makes the rest of the page inert: a popup in the body would be behind it
    // and could not be reached with the pointer. While a hint of the dialog is open the popup lives in the dialog.
    const parent = el.closest('dialog') ?? doc.body;
    if (host.parentElement !== parent) parent.appendChild(host);
    host.hidden = false;
    tip.hidden = false;
    if (!ownsDescription(el)) el.setAttribute('aria-describedby', withToken(describedBefore, TIP_ID));
    place(el);
  }

  const onOver = (e: Event): void => {
    if (pinned) return;
    if (e.target instanceof win.Node && tip.contains(e.target)) {
      cancelHide();
      return;
    }
    const el = hintOf(e.target);
    if (!el) {
      cancelShow();
      // A popup the focus opened stays while the focus does (WCAG 1.4.13): a pointer that rests where a dialog moved to
      // gets a mouseover from the browser, and that is no reason to take the keyboard's hint away.
      if (!tip.hidden && !viaFocus) scheduleHide();
      return;
    }
    cancelHide();
    if (el === current) return;
    // The pointer is on another element: the popup of the one it left goes at once, it does not wait for the next.
    if (current && !tip.hidden) hide();
    cancelShow();
    showTimer = win.setTimeout(() => show(el, false), hintDelay(el.dataset.hintDelay));
  };
  const onOut = (e: Event): void => {
    if (pinned) return;
    cancelShow();
    if (!tip.hidden && !viaFocus && (hintOf(e.target) || (e.target instanceof win.Node && tip.contains(e.target)))) {
      scheduleHide();
    }
  };
  // A press of the mouse focuses what it hits, and the browser may scroll it into view while it does: a popup shown by that
  // focus would be placed for the element before the scroll and lie under the pointer, where it catches the click. A press
  // shows no popup; the pointer resting on the element does (hover), and the keyboard focus does at once.
  // A press inside the popup keeps it open until a press outside (the text can be selected and copied); a press outside
  // takes a kept popup away. A popup on a control never gets the press (pointer-events: none).
  const onDown = (e: Event): void => {
    pressing = true;
    // The main button only: another one opens a menu of the browser.
    if ((e as MouseEvent).button !== 0 || tip.hidden) return;
    if (e.target instanceof win.Node && tip.contains(e.target)) {
      pinned = true;
      cancelHide();
    } else if (pinned) hide();
  };
  const onUp = (): void => {
    pressing = false;
  };
  const onFocusIn = (e: Event): void => {
    if (pressing || pinned) return;
    const el = hintOf(e.target);
    if (el) show(el, true);
  };
  const onFocusOut = (): void => {
    if (!pinned) hide();
  };
  // A tap shows the popup at once: a touch screen has no hover to wait for, and not every browser focuses what is
  // tapped (a button on iOS). A click with a mouse or the keyboard does not: hover and focus did it already.
  const onClick = (e: Event): void => {
    if ((e as PointerEvent).pointerType !== 'touch') return;
    if (e.target instanceof win.Node && tip.contains(e.target)) return;
    const el = hintOf(e.target);
    if (el) {
      cancelShow();
      show(el, false);
    } else hide();
  };
  const onChange: Change = (el, gone) => {
    if (el !== current) return;
    if (gone || (el.dataset.hint ?? '') === '') hide();
    else show(el, viaFocus);
  };
  watchers.add(onChange);
  const onKey = (e: KeyboardEvent): void => {
    // A key after a press that never got its release (the button was let go outside the window) is the keyboard again.
    pressing = false;
    // Tab is the keyboard again: a kept popup is a kept one no longer (Ctrl+C copies from it, Escape closes it below).
    if (e.key === 'Tab' && pinned) hide();
    // The popup is closed by this Escape and by nothing else: it is not also the Escape of a dialog or a menu. A
    // modal dialog closes on Escape by its own default action (`cancel`), which `stopPropagation` does not stop.
    if (e.key === 'Escape' && !tip.hidden) {
      hide();
      e.stopPropagation();
      e.preventDefault();
    }
  };
  const onDialogClose = (e: Event): void => {
    if (pinned && e.target instanceof win.Node && e.target.contains(host)) hide();
  };
  const onScroll = (): void => {
    if (tip.hidden) return;
    if (pinned) {
      if (current?.isConnected) place(current, true);
      else hide();
      return;
    }
    // The browser scrolls a focused element into view after `focusin`: the popup follows it. A scroll with the
    // pointer takes the popup away (it would stand where the element no longer is).
    if (viaFocus && current) place(current, true);
    else hide();
  };

  doc.addEventListener('mouseover', onOver);
  doc.addEventListener('mouseout', onOut);
  doc.addEventListener('mousedown', onDown, true);
  doc.addEventListener('mouseup', onUp, true);
  doc.addEventListener('focusin', onFocusIn);
  doc.addEventListener('focusout', onFocusOut);
  doc.addEventListener('keydown', onKey);
  doc.addEventListener('click', onClick);
  doc.addEventListener('scroll', onScroll, true);
  doc.addEventListener('close', onDialogClose, true);
  return () => {
    hide();
    watchers.delete(onChange);
    doc.removeEventListener('click', onClick);
    doc.removeEventListener('mouseover', onOver);
    doc.removeEventListener('mouseout', onOut);
    doc.removeEventListener('mousedown', onDown, true);
    doc.removeEventListener('mouseup', onUp, true);
    doc.removeEventListener('focusin', onFocusIn);
    doc.removeEventListener('focusout', onFocusOut);
    doc.removeEventListener('keydown', onKey);
    doc.removeEventListener('scroll', onScroll, true);
    doc.removeEventListener('close', onDialogClose, true);
    host.remove();
  };
}
