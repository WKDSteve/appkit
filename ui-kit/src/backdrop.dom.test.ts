// @vitest-environment jsdom
// A dialog that closes on a click beside it, in a real DOM. The box is mocked (jsdom has no layout): it is 100 x 100
// at 50, 50, so (10, 10) is the backdrop and (80, 80) is the dialog. jsdom does not know `closedby`: the listeners are
// the way of such a browser; a browser that knows it is stood in for by a `closedBy` on the prototype. What the browser
// then does itself (a click beside the dialog closes it, a selection out of it does not) is shown in Chromium by
// `ui-kit/browser/backdrop.browser.cjs`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { closeOnBackdrop, hasClosedBy, isOutside, type BackdropOptions } from './backdrop';

const box = { left: 50, right: 150, top: 50, bottom: 150 };

function page(options?: BackdropOptions) {
  document.body.innerHTML = '<dialog open><p id="inside">text</p><button id="btn">ok</button></dialog>';
  const dialog = document.querySelector('dialog') as HTMLDialogElement;
  dialog.getBoundingClientRect = () => ({ ...box, x: 50, y: 50, width: 100, height: 100, toJSON: () => ({}) });
  const close = vi.fn(() => dialog.removeAttribute('open'));
  dialog.close = close;
  const action = closeOnBackdrop(dialog, options);
  const at = (el: Element, type: string, x: number, y: number) =>
    el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y }));
  const click = (el: Element, x: number, y: number) => {
    at(el, 'mousedown', x, y);
    at(el, 'mouseup', x, y);
    at(el, 'click', x, y);
  };
  return { dialog, close, action, at, click };
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('isOutside', () => {
  it('is false on the box, its edge included, and true beside it', () => {
    expect(isOutside(box, 80, 80)).toBe(false);
    expect(isOutside(box, 50, 150)).toBe(false);
    expect(isOutside(box, 49, 80)).toBe(true);
    expect(isOutside(box, 80, 151)).toBe(true);
  });
});

describe('closeOnBackdrop without closedby', () => {
  it('runs in a DOM that does not know closedby', () => {
    expect(hasClosedBy()).toBe(false);
    const { dialog } = page();
    expect(dialog.hasAttribute('closedby')).toBe(false);
  });

  it('closes the dialog for a click on the backdrop', () => {
    const { dialog, close, click } = page();
    click(dialog, 10, 10);
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('keeps it for a click on the dialog itself (its padding) and on what is inside it', () => {
    const { dialog, close, click } = page();
    click(dialog, 80, 80);
    click(document.getElementById('inside') as Element, 80, 80);
    click(document.getElementById('btn') as Element, 90, 90);
    expect(close).not.toHaveBeenCalled();
  });

  it('keeps it when the press started inside (a selection that ends on the backdrop)', () => {
    const { dialog, close, at } = page();
    at(dialog, 'mousedown', 80, 80);
    at(dialog, 'click', 10, 10);
    expect(close).not.toHaveBeenCalled();
  });

  it('asks closeable first, and keeps the dialog on false', () => {
    const closeable = vi.fn(() => false);
    const { dialog, close, click, action } = page({ closeable });
    click(dialog, 10, 10);
    expect(closeable).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();
    action.update({ closeable: () => true });
    click(dialog, 10, 10);
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('sends the cancel event first, like Escape: a dialog that prevents it stays open', () => {
    const { dialog, close, click } = page();
    const seen = vi.fn((e: Event) => e.preventDefault());
    dialog.addEventListener('cancel', seen);
    click(dialog, 10, 10);
    expect(seen).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();
    dialog.removeEventListener('cancel', seen);
    click(dialog, 10, 10);
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('does nothing for a dialog that is not open, and after destroy', () => {
    const { dialog, close, click, action } = page();
    dialog.removeAttribute('open');
    click(dialog, 10, 10);
    expect(close).not.toHaveBeenCalled();
    dialog.setAttribute('open', '');
    action.destroy();
    click(dialog, 10, 10);
    expect(close).not.toHaveBeenCalled();
  });
});

describe('closeOnBackdrop where the browser knows closedby', () => {
  beforeEach(() => {
    Object.defineProperty(HTMLDialogElement.prototype, 'closedBy', { configurable: true, value: 'auto' });
  });
  afterEach(() => {
    delete (HTMLDialogElement.prototype as { closedBy?: string }).closedBy;
  });

  it('sets closedby="any" and leaves the click to the browser', () => {
    expect(hasClosedBy()).toBe(true);
    const { dialog, close, click } = page();
    expect(dialog.getAttribute('closedby')).toBe('any');
    const seen = vi.fn();
    dialog.addEventListener('cancel', seen);
    click(dialog, 10, 10);
    expect(seen).not.toHaveBeenCalled();
    expect(close).not.toHaveBeenCalled();
  });

  it('keeps the listeners for a dialog with closeable, and moves between the ways on update', () => {
    const closeable = vi.fn(() => false);
    const { dialog, close, click, action } = page({ closeable });
    expect(dialog.hasAttribute('closedby')).toBe(false);
    click(dialog, 10, 10);
    expect(closeable).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();

    action.update({});
    expect(dialog.getAttribute('closedby')).toBe('any');
    click(dialog, 10, 10);
    expect(closeable).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();

    action.update({ closeable: () => true });
    expect(dialog.hasAttribute('closedby')).toBe(false);
    click(dialog, 10, 10);
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('takes the attribute away on destroy', () => {
    const { dialog, action } = page();
    action.destroy();
    expect(dialog.hasAttribute('closedby')).toBe(false);
  });
});
