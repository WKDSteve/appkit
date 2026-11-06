// @vitest-environment jsdom
// The popup in a real DOM: a press of the mouse focuses what it hits (the browser does it as the default action of
// mousedown), and the popup that focus would show lay under the pointer once the browser had scrolled the element into
// view. The press shows nothing; the keyboard focus does.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HIDE_DELAY_MS, installTooltips, SHOW_DELAY_MS, TIP_ID } from './tooltip';

let uninstall: (() => void) | undefined;

function page() {
  document.body.innerHTML = '<button id="a" data-hint="First">A</button><button id="b" data-hint="Second">B</button>';
  uninstall = installTooltips(document, window, { regionLabel: () => 'Hints' });
  const a = document.getElementById('a') as HTMLButtonElement;
  const b = document.getElementById('b') as HTMLButtonElement;
  // What the browser does with a press on a button: it focuses it, between mousedown and mouseup.
  for (const el of [a, b]) el.addEventListener('mousedown', () => el.focus());
  const tip = document.getElementById(TIP_ID) as HTMLElement;
  const fire = (el: Element, type: string) => el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true }));
  return { a, b, tip, fire };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  uninstall?.();
  uninstall = undefined;
  vi.useRealTimers();
  document.body.innerHTML = '';
});

describe('the popup of the keyboard focus', () => {
  it('stays when the pointer moves on something without a hint, and goes with the focus', () => {
    const { a, tip, fire } = page();
    a.focus();
    expect(tip.hidden).toBe(false);
    fire(document.body, 'mouseover');
    fire(a, 'mouseout');
    vi.advanceTimersByTime(HIDE_DELAY_MS + 1);
    expect(tip.hidden).toBe(false);
    a.blur();
    expect(tip.hidden).toBe(true);
  });

  it('of the pointer goes when the pointer leaves, as before', () => {
    const { a, tip, fire } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.hidden).toBe(false);
    fire(document.body, 'mouseover');
    vi.advanceTimersByTime(HIDE_DELAY_MS + 1);
    expect(tip.hidden).toBe(true);
  });
});

describe('the popup and the press of the mouse', () => {
  it('shows nothing for the focus of a press, also when the popup of another element is warm', () => {
    const { a, b, tip, fire } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.hidden).toBe(false);
    expect(tip.textContent).toBe('First');
    // The pointer goes to the second button and presses it: its focus follows, and the popup of the first goes with the
    // focus it had.
    a.focus();
    fire(b, 'mousedown');
    expect(document.activeElement).toBe(b);
    expect(tip.hidden).toBe(true);
    fire(b, 'mouseup');
    expect(tip.hidden).toBe(true);
  });

  it('still shows at once for the focus of the keyboard', () => {
    const { a, b, tip, fire } = page();
    fire(a, 'mousedown');
    fire(a, 'mouseup');
    a.blur();
    b.focus();
    expect(tip.hidden).toBe(false);
    expect(tip.textContent).toBe('Second');
  });
});

describe('the popup that is pressed in, and the one that lies on a control', () => {
  it('stays open after a press inside it, also when the pointer and the focus leave, until a press outside', () => {
    const { a, tip, fire } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    a.focus();
    fire(tip, 'mousedown');
    fire(tip, 'mouseup');
    // The pointer goes away and the focus with it (a press on text that cannot be focused takes the focus to the page).
    fire(tip, 'mouseout');
    a.blur();
    vi.advanceTimersByTime(HIDE_DELAY_MS * 4);
    expect(tip.hidden).toBe(false);
    fire(document.body, 'mousedown');
    expect(tip.hidden).toBe(true);
  });

  it('is taken away by Escape also when it was pressed in', () => {
    const { a, tip, fire } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    fire(tip, 'mousedown');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    expect(tip.hidden).toBe(true);
  });

  it('goes at once when the pointer is on another element with a hint, not after the pause of the next', () => {
    const { a, b, tip, fire } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.hidden).toBe(false);
    fire(a, 'mouseout');
    fire(b, 'mouseover');
    expect(tip.hidden).toBe(true);
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.textContent).toBe('Second');
  });

  it('lets the pointer through when it lies on a control, and not when it has a free place', () => {
    const { a, b, tip, fire } = page();
    const rect = (x: number, y: number, width: number, height: number) =>
      ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON: () => ({}) }) as DOMRect;
    a.getBoundingClientRect = () => rect(10, 10, 50, 20);
    tip.getBoundingClientRect = () => rect(0, 0, 100, 30);
    // The second button is under every place the popup could take.
    b.getBoundingClientRect = () => rect(0, 0, 5000, 5000);
    document.elementsFromPoint = () => [b];
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.hidden).toBe(false);
    expect(tip.style.pointerEvents).toBe('none');
    // Nothing under it any more: the popup can be reached and pressed in.
    fire(a, 'mouseout');
    vi.advanceTimersByTime(HIDE_DELAY_MS);
    b.getBoundingClientRect = () => rect(0, 0, 0, 0);
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.hidden).toBe(false);
    expect(tip.style.pointerEvents).toBe('');
  });

  it('stays on a tap inside it, and goes with Tab (the focus is on the page, no focusout comes)', () => {
    const { a, tip, fire } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    fire(tip, 'mousedown');
    tip.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
    expect(tip.hidden).toBe(false);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    expect(tip.hidden).toBe(true);
  });

  it('goes with the dialog it lives in when that closes, so that the hints of the page work again', () => {
    document.body.innerHTML = '<dialog id="d" open><button id="x" data-hint="In dialog">X</button></dialog><button id="y" data-hint="Next">Y</button>';
    uninstall = installTooltips(document, window, { regionLabel: () => 'Hints' });
    const x = document.getElementById('x') as HTMLElement;
    const y = document.getElementById('y') as HTMLElement;
    const tip = document.getElementById(TIP_ID) as HTMLElement;
    const fire = (el: Element, type: string) => el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true }));
    fire(x, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    fire(tip, 'mousedown');
    document.getElementById('d')!.dispatchEvent(new Event('close'));
    expect(tip.hidden).toBe(true);
    fire(y, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.textContent).toBe('Next');
  });
});
