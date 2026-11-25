// A modal dialog with something typed into it does not close on a stray Escape or a click beside it: it asks first. The
// action sets `closedby="none"` (a browser that knows it then closes the dialog by no gesture of its own), answers
// Escape itself (asks when `dirty()`, else closes), and keeps the `cancel` event as the way out for a browser that has no
// `closedby` and fires `cancel` for Escape (it is prevented while `dirty()`). The click beside the dialog stays with
// `closeOnBackdrop({ closeable })`, which ignores it while there is something typed.

export interface LeaveOptions {
  /** Whether closing now would lose something the person typed or chose. */
  dirty: () => boolean;
  /** Show the question "discard what you entered?". */
  ask: () => void;
}

export function leaveGuard(node: HTMLDialogElement, options: LeaveOptions) {
  let opts = options;
  node.setAttribute('closedby', 'none');

  // On the document, not on the dialog: whatever inside the dialog takes an Escape for its own (a popup of a hint, an open
  // list) has handled it by then and marks it (`defaultPrevented`), and the document's listener of the kit runs before this one.
  const doc = node.ownerDocument;
  const onKey = (e: KeyboardEvent): void => {
    if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing) return;
    if (!node.open || !(e.target instanceof Node) || !node.contains(e.target)) return;
    e.preventDefault();
    if (opts.dirty()) opts.ask();
    else node.close();
  };
  const onCancel = (e: Event): void => {
    if (!opts.dirty()) return;
    e.preventDefault();
    opts.ask();
  };
  doc.addEventListener('keydown', onKey);
  node.addEventListener('cancel', onCancel);
  return {
    update(next: LeaveOptions): void {
      opts = next;
    },
    destroy(): void {
      doc.removeEventListener('keydown', onKey);
      node.removeEventListener('cancel', onCancel);
      node.removeAttribute('closedby');
    },
  };
}
