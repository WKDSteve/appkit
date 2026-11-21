// A modal <dialog> closes when the pointer is pressed and released on the backdrop beside it, as it does for Escape.
// The backdrop is part of the dialog element for the events: a click on it has the dialog itself as its target, so
// the click counts only when it lies outside the box of the dialog (a click on the padding of the box is inside).
//
// A browser that knows `closedby` (Chrome/Edge 134, Firefox 141; Limited availability, not Safari yet) does the same itself with `closedby="any"`: a press and release on the backdrop sends a cancelable
// `cancel` and then closes; a press in the dialog that ends on the backdrop (a text selection) does not. There the
// action only sets the attribute. Not with `closeable`: the browser's `cancel` reaches the dialog's own `oncancel`
// before anything could ask, and it cannot be told from the `cancel` of Escape, which `closeable` must not keep;
// such a dialog keeps the listeners below.

/** Where the box of a dialog is, as far as the check needs it. */
export interface Rect {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** True when the point lies outside the box: on the backdrop, not on the dialog. */
export function isOutside(rect: Rect, x: number, y: number): boolean {
  return x < rect.left || x > rect.right || y < rect.top || y > rect.bottom;
}

export interface BackdropOptions {
  /** Asked before the dialog closes; `false` keeps it open (a dialog with a draft that Escape would also lose). */
  closeable?: () => boolean;
}

/** True when the browser closes a modal dialog on its backdrop by itself (`closedby="any"`). */
export function hasClosedBy(): boolean {
  return typeof HTMLDialogElement !== 'undefined' && 'closedBy' in HTMLDialogElement.prototype;
}

/**
 * `use:closeOnBackdrop` on a `<dialog>` opened with `showModal()`. Both the press and the release must be on the
 * backdrop: a text selection that starts in the dialog and ends outside it must not close it. It goes the way Escape
 * goes: a `cancel` event first (a dialog that handles `oncancel` itself and calls `preventDefault()` - the conflict
 * center asks the app - keeps it), then `close()`, so the `onclose` handler of the dialog runs as it does for Escape.
 * Where the browser knows `closedby` and no `closeable` is given, the action sets `closedby="any"` (and owns that
 * attribute) instead of listening; the browser then does the same.
 */
export function closeOnBackdrop(node: HTMLDialogElement, options: BackdropOptions = {}) {
  let opts = options;
  let pressedOutside = false;
  let way: 'closedby' | 'listeners' | null = null;

  const outside = (e: MouseEvent) => e.target === node && isOutside(node.getBoundingClientRect(), e.clientX, e.clientY);

  const onDown = (e: MouseEvent) => {
    pressedOutside = outside(e);
  };
  const onClick = (e: MouseEvent) => {
    const start = pressedOutside;
    pressedOutside = false;
    if (!start || !outside(e) || !node.open) return;
    if (opts.closeable && !opts.closeable()) return;
    const cancel = new Event('cancel', { cancelable: true });
    node.dispatchEvent(cancel);
    if (!cancel.defaultPrevented) node.close();
  };

  const leave = () => {
    if (way === 'closedby') node.removeAttribute('closedby');
    if (way === 'listeners') {
      node.removeEventListener('mousedown', onDown);
      node.removeEventListener('click', onClick);
    }
    way = null;
  };
  // The way can change with the options: a dialog that gets a `closeable` later moves to the listeners.
  const choose = () => {
    const next = hasClosedBy() && !opts.closeable ? 'closedby' : 'listeners';
    if (next === way) return;
    leave();
    way = next;
    if (way === 'closedby') {
      node.setAttribute('closedby', 'any');
    } else {
      pressedOutside = false;
      node.addEventListener('mousedown', onDown);
      node.addEventListener('click', onClick);
    }
  };

  choose();
  return {
    update(next: BackdropOptions = {}) {
      opts = next;
      choose();
    },
    destroy() {
      leave();
    },
  };
}
