// @vitest-environment jsdom
// Rule APG-06 and WCAG 1.4.13 (content on hover or focus): the popup shows on focus and on hover, is hoverable
// (stays while the pointer is on it), is dismissible with Escape without moving the focus, and persists until the
// focus or the pointer leaves. It is a `role="tooltip"` that describes its element.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HIDE_DELAY_MS, installTooltips, SHOW_DELAY_MS, TIP_ID } from './tooltip';

let uninstall: (() => void) | undefined;

function page() {
  document.body.innerHTML = '<button id="a" data-hint="First hint">A</button><button id="b">B</button>';
  uninstall = installTooltips(document, window, { regionLabel: () => 'Hints' });
  const a = document.getElementById('a') as HTMLButtonElement;
  const tip = document.getElementById(TIP_ID) as HTMLElement;
  const fire = (el: Element, type: string) => el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true }));
  const tab = () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
  const escape = () => {
    const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    document.activeElement?.dispatchEvent(e);
    return e;
  };
  return { a, tip, fire, tab, escape };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  uninstall?.();
  uninstall = undefined;
  vi.useRealTimers();
  document.body.innerHTML = '';
});

describe('APG-06 tooltip', () => {
  it('is a tooltip that describes its element, with the text of the hint', () => {
    const { a, tip, tab } = page();
    expect(tip.getAttribute('role')).toBe('tooltip');
    tab();
    a.focus();
    expect(tip.hidden).toBe(false);
    expect(tip.textContent).toBe('First hint');
    expect(a.getAttribute('aria-describedby')).toContain(TIP_ID);
  });

  it('shows at once on keyboard focus and goes with the blur (no timeout of its own)', () => {
    const { a, tip, tab } = page();
    tab();
    a.focus();
    expect(tip.hidden).toBe(false);
    vi.advanceTimersByTime(60_000);
    expect(tip.hidden).toBe(false); // persistent: stays until the focus leaves
    a.blur();
    expect(tip.hidden).toBe(true);
    expect(a.hasAttribute('aria-describedby')).toBe(false);
  });

  it('shows on hover after the short pause, and goes when the pointer leaves', () => {
    const { a, tip, fire } = page();
    fire(a, 'mouseover');
    expect(tip.hidden).toBe(true);
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.hidden).toBe(false);
    fire(a, 'mouseout');
    fire(document.body, 'mouseover');
    vi.advanceTimersByTime(HIDE_DELAY_MS + 1);
    expect(tip.hidden).toBe(true);
  });

  it('is hoverable: it stays while the pointer moves from the element onto the popup, and goes when it leaves that', () => {
    const { a, tip, fire } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    // The browser sends mouseout for the element, then mouseover for the popup.
    fire(a, 'mouseout');
    fire(tip, 'mouseover');
    vi.advanceTimersByTime(HIDE_DELAY_MS * 10);
    expect(tip.hidden).toBe(false);
    fire(tip, 'mouseout');
    fire(document.body, 'mouseover');
    vi.advanceTimersByTime(HIDE_DELAY_MS + 1);
    expect(tip.hidden).toBe(true);
  });

  it('Escape closes it without moving the focus, and the key does not reach the page behind', () => {
    const { a, tip, tab, escape } = page();
    const behind = vi.fn();
    tab();
    a.focus();
    window.addEventListener('keydown', behind);
    const e = escape();
    expect(tip.hidden).toBe(true);
    expect(document.activeElement).toBe(a);
    expect(e.defaultPrevented).toBe(true);
    expect(behind).not.toHaveBeenCalled();
    window.removeEventListener('keydown', behind);
    // A second Escape (no popup open) is the page's again.
    const second = escape();
    expect(second.defaultPrevented).toBe(false);
  });

  it('Escape also closes the popup of the pointer, with no focus on the element', () => {
    const { a, tip, fire, escape } = page();
    fire(a, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS);
    expect(tip.hidden).toBe(false);
    escape();
    expect(tip.hidden).toBe(true);
  });

  it('an element without a hint shows nothing on focus or hover', () => {
    const { tip, fire, tab } = page();
    const b = document.getElementById('b') as HTMLButtonElement;
    tab();
    b.focus();
    fire(b, 'mouseover');
    vi.advanceTimersByTime(SHOW_DELAY_MS * 2);
    expect(tip.hidden).toBe(true);
  });
});
