// A float the person can move and size: `use:floating` on a modal <dialog>. The dialog stays what it was (modal, the
// focus kept inside, Escape closes it); this adds
//
// - a handle to move it: the dialog's own (an element with `data-float-move`, and a head with `data-float-handle`) or
//   a small button this adds in its upper right corner; the title the dialog is named by (`aria-labelledby`) is a
//   handle too. A drag of a handle moves the float; on the button, the arrow keys move it, Shift with the arrow keys
//   changes its size, Home (or a double click) puts it back where the browser puts a dialog;
// - the browser's grip in the lower right corner (CSS `resize`) to change its size with the mouse.
//
// Where it was put and how big it was made is kept per kind of float in this browser (floats.ts) and given back when
// the float opens again. The float never leaves the viewport: a kept place is fitted into the window it opens in, and
// a window that gets smaller pushes it back in.

import {
  clampBox,
  FLOAT_MARGIN,
  keyedBox,
  loadPlace,
  movedBox,
  savePlace,
  sizedBox,
  type Box,
  type PlaceStore,
  type Viewport,
} from './floats';

export interface FloatingParams {
  /** The kind of float: its place is kept under this name. */
  kind: string;
  /** The name of the handle this adds (unused when the float brings its own). */
  moveLabel: string;
  /** What the handle does, for the hint popup of the page. */
  moveHint: string;
  /** False when the float brings its own handle: an element with `data-float-move`. */
  handle?: boolean;
}

/** How near the lower right corner a press counts as one on the browser's grip, in pixels. */
const GRIP = 20;

/** The elements a press on which is a press on a control, not on the head around it. */
const CONTROLS = 'button, a, input, select, textarea, summary, label, [contenteditable="true"]';

/** The controls that take the focus with Tab (the first of them is where a dialog that opens puts it). */
const FOCUSABLE =
  'button:not([disabled]), a[href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

const STYLES = ['margin', 'left', 'top', 'right', 'bottom', 'width', 'height', 'max-width', 'max-height', 'box-sizing'];

function browserStore(): PlaceStore | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** The action of an app: its floats keep their places under `storeKey` in the browser's storage. */
export function floatingIn(storeKey: string) {
  return (node: HTMLDialogElement, params: FloatingParams) => floatingUnder(storeKey, node, params);
}

/** The action a dialog of the kit is given by the app (see `floatingIn`). */
export type FloatAction = ReturnType<typeof floatingIn>;

function floatingUnder(storeKey: string, node: HTMLDialogElement, params: FloatingParams) {
  let current = params;
  let added: HTMLButtonElement | null = null;
  let wasOpen = false;
  /** The drag under way: of a handle (`move`), or of the browser's grip (`grip`, followed only to its release). */
  let drag: { mode: 'move'; id: number; sx: number; sy: number; start: Box } | { mode: 'grip'; start: Box } | null = null;

  const view = (): Viewport => ({ w: window.innerWidth, h: window.innerHeight });
  const measure = (): Box => {
    const r = node.getBoundingClientRect();
    return { x: r.left, y: r.top, w: r.width, h: r.height };
  };
  /** Whether the float has a size of its own (else its content gives it one). */
  const sized = () => node.style.width !== '' && node.style.height !== '';

  /** Put the top left corner at `box.x`/`box.y`; with `withSize`, give it the size of the box too. */
  function put(box: Box, withSize: boolean): void {
    const s = node.style;
    const v = view();
    s.margin = '0';
    s.left = `${box.x}px`;
    s.top = `${box.y}px`;
    s.right = 'auto';
    s.bottom = 'auto';
    // The content may grow after the float is placed (a task's text arrives): it scrolls inside, the box stays in view.
    s.maxWidth = `${Math.max(0, v.w - FLOAT_MARGIN - box.x)}px`;
    s.maxHeight = `${Math.max(0, v.h - FLOAT_MARGIN - box.y)}px`;
    if (withSize) {
      s.boxSizing = 'border-box';
      s.width = `${box.w}px`;
      s.height = `${box.h}px`;
    }
  }

  function unplace(): void {
    for (const name of STYLES) node.style.removeProperty(name);
  }

  /** The place kept for this kind, fitted into the window; none kept: where the browser puts a dialog. */
  function restore(): void {
    unplace();
    const kept = loadPlace(browserStore(), storeKey, current.kind);
    if (!kept) return;
    const natural = measure();
    const withSize = kept.w !== undefined && kept.h !== undefined;
    const box = clampBox({ x: kept.x, y: kept.y, w: kept.w ?? natural.w, h: kept.h ?? natural.h }, view());
    put(box, withSize);
  }

  function keep(): void {
    const r = measure();
    const place = { x: Math.round(r.x), y: Math.round(r.y) };
    savePlace(browserStore(), storeKey, current.kind, sized() ? { ...place, w: Math.round(r.w), h: Math.round(r.h) } : place);
  }

  function reset(): void {
    savePlace(browserStore(), storeKey, current.kind, null);
    unplace();
  }

  function markHead(): void {
    const id = node.getAttribute('aria-labelledby');
    if (id) node.querySelector(`#${CSS.escape(id)}`)?.classList.add('float-head');
  }

  function handleOf(target: Element): Element | null {
    const own = target.closest('[data-float-move], [data-float-handle], .float-head');
    return own && node.contains(own) ? own : null;
  }

  function down(e: PointerEvent): void {
    if (e.button !== 0 || !node.open || !(e.target instanceof Element)) return;
    const target = e.target;
    const handle = handleOf(target);
    if (handle) {
      // A button in the head is a button, not the head.
      const control = target.closest(CONTROLS);
      if (control && control !== handle && node.contains(control)) return;
      e.preventDefault();
      drag = { mode: 'move', id: e.pointerId, sx: e.clientX, sy: e.clientY, start: measure() };
      try {
        handle.setPointerCapture(e.pointerId);
      } catch {
        // A pointer that is gone cannot be captured; the drag then ends with the next release.
      }
      return;
    }
    // The browser's grip: the float's top left corner is fixed first, so that the corner follows the mouse (a dialog
    // in the middle would grow to both sides). What it became is read when the mouse is let go.
    const r = measure();
    if (target === node && e.clientX >= r.x + r.w - GRIP && e.clientY >= r.y + r.h - GRIP) {
      put(r, true);
      drag = { mode: 'grip', start: r };
    }
  }

  function move(e: PointerEvent): void {
    if (drag?.mode !== 'move' || e.pointerId !== drag.id) return;
    put(movedBox(drag.start, e.clientX - drag.sx, e.clientY - drag.sy, view()), false);
  }

  function up(e: PointerEvent): void {
    if (drag === null) return;
    // Closed during the drag (Escape): a closed dialog measures nothing, and that must not become its place.
    if (!node.open) {
      drag = null;
      return;
    }
    if (drag.mode === 'move') {
      if (e.pointerId !== drag.id) return;
      const start = drag.start;
      drag = null;
      const now = measure();
      if (Math.abs(now.x - start.x) >= 1 || Math.abs(now.y - start.y) >= 1) keep();
      return;
    }
    const start = drag.start;
    drag = null;
    const now = measure();
    if (Math.abs(now.w - start.w) > 1 || Math.abs(now.h - start.h) > 1) {
      // The grip went past the window: the float is fitted back in.
      put(clampBox(now, view()), true);
      keep();
    } else {
      // A press near the corner without a drag changes nothing: the float goes back to what it was.
      restore();
    }
  }

  function key(e: KeyboardEvent): void {
    if (!(e.target instanceof Element) || !e.target.closest('[data-float-move]')) return;
    if (e.key === 'Home') {
      e.preventDefault();
      reset();
      return;
    }
    const next = keyedBox(measure(), e.key, e.shiftKey, view());
    if (next === null) return;
    e.preventDefault();
    put(next.box, next.resized || sized());
    keep();
  }

  function doubleClick(e: MouseEvent): void {
    if (e.target instanceof Element && e.target.closest('[data-float-move]')) reset();
  }

  /** A window that got smaller pushes a placed float back into it (nothing is kept: the place the person chose stays). */
  function fit(): void {
    if (!node.open || node.style.left === '') return;
    put(clampBox(measure(), view()), sized());
  }

  function label(): void {
    if (!added) return;
    added.setAttribute('aria-label', current.moveLabel);
    added.setAttribute('data-hint', current.moveHint);
  }

  /**
   * The browser gives a dialog that opens the focus on its first control, which is the handle this added: it goes on to
   * the control after it, where it went before there was a handle (a dialog that puts the focus itself is not touched).
   */
  function skipHandle(): void {
    if (!added || document.activeElement !== added) return;
    const next = [...node.querySelectorAll<HTMLElement>(FOCUSABLE)].find(
      (el) => el !== added && el.getClientRects().length > 0,
    );
    next?.focus();
  }

  node.classList.add('floating');
  if (current.handle !== false) {
    added = document.createElement('button');
    added.type = 'button';
    added.className = 'float-move';
    added.setAttribute('data-float-move', '');
    const icon = document.createElement('span');
    icon.setAttribute('aria-hidden', 'true');
    icon.textContent = '✥';
    added.append(icon);
    label();
    // First in the order of the focus, as it is drawn first: in the upper right corner, at the head of the float.
    node.prepend(added);
  }

  const watch = new MutationObserver(() => {
    if (node.open && !wasOpen) {
      restore();
      skipHandle();
    }
    wasOpen = node.open;
    // The title may be another element by now (a dialog of two steps draws a heading for each): it is a handle too.
    if (node.open) markHead();
  });
  // The open state, and the content: a heading drawn while the dialog stays open.
  watch.observe(node, { attributes: true, attributeFilter: ['open'], childList: true, subtree: true });
  if (node.open) {
    wasOpen = true;
    markHead();
    restore();
  }

  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', up);
  node.addEventListener('keydown', key);
  node.addEventListener('dblclick', doubleClick);
  // The grip's release may happen outside the float.
  window.addEventListener('pointerup', up);
  window.addEventListener('resize', fit);

  return {
    update(next: FloatingParams): void {
      current = next;
      label();
    },
    destroy(): void {
      watch.disconnect();
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', up);
      node.removeEventListener('keydown', key);
      node.removeEventListener('dblclick', doubleClick);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('resize', fit);
      added?.remove();
    },
  };
}
