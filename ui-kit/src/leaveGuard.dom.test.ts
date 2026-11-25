// @vitest-environment jsdom
// A dialog with something typed in it asks before Escape closes it; an empty one closes at once.
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { leaveGuard } from './leaveGuard';

beforeAll(() => {
  const proto = HTMLDialogElement.prototype as unknown as { showModal?: () => void; close?: () => void };
  proto.showModal ??= function (this: HTMLDialogElement) {
    this.setAttribute('open', '');
  };
  proto.close ??= function (this: HTMLDialogElement) {
    this.removeAttribute('open');
  };
});

afterEach(() => {
  document.body.innerHTML = '';
});

function open(dirty: boolean): { dialog: HTMLDialogElement; asked: () => number; stop: () => void } {
  const dialog = document.createElement('dialog');
  document.body.append(dialog);
  dialog.showModal();
  let asked = 0;
  const action = leaveGuard(dialog, { dirty: () => dirty, ask: () => (asked += 1) });
  return { dialog, asked: () => asked, stop: () => action.destroy() };
}

const escape = (dialog: HTMLElement): KeyboardEvent => {
  const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
  dialog.dispatchEvent(event);
  return event;
};

describe('leaveGuard', () => {
  it('sets closedby="none" and takes it away again', () => {
    const { dialog, stop } = open(false);
    expect(dialog.getAttribute('closedby')).toBe('none');
    stop();
    expect(dialog.hasAttribute('closedby')).toBe(false);
  });

  it('asks on Escape while something is typed, and keeps the dialog open', () => {
    const { dialog, asked } = open(true);
    const event = escape(dialog);
    expect(asked()).toBe(1);
    expect(event.defaultPrevented).toBe(true);
    expect(dialog.open).toBe(true);
  });

  it('closes on Escape when nothing is typed', () => {
    const { dialog, asked } = open(false);
    escape(dialog);
    expect(asked()).toBe(0);
    expect(dialog.open).toBe(false);
  });

  it('prevents the cancel of a browser that fires it for Escape while something is typed', () => {
    const { dialog, asked } = open(true);
    const cancel = new Event('cancel', { cancelable: true });
    dialog.dispatchEvent(cancel);
    expect(cancel.defaultPrevented).toBe(true);
    expect(asked()).toBe(1);
  });

  it('leaves an Escape that something inside already handled', () => {
    const { dialog, asked } = open(true);
    const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    event.preventDefault();
    dialog.dispatchEvent(event);
    expect(asked()).toBe(0);
  });
});
