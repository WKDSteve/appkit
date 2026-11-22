// @vitest-environment jsdom
// Rule APG-08 (multiple choice; WCAG 2.1.1, 2.4.3, 4.1.2). The choice is a button that discloses a group of checkboxes
// (not a listbox): the key sequence is that of the disclosure with a checkbox group, plus the arrows of a list.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import MultiSelect from './MultiSelect.svelte';

let app: ReturnType<typeof mount> | undefined;
let selected: string[] = [];
const options = Array.from({ length: 12 }, (_, i) => ({ value: `v${i}`, label: `Option ${i}` }));

beforeEach(() => {
  selected = [];
  document.body.innerHTML = '<button id="before">Before</button>';
  app = mount(MultiSelect, {
    target: document.body,
    props: { label: 'Areas', options, selected, onchange: (n: string[]) => (selected = n), resetLabel: 'Reset', searchLabel: 'Search' },
  });
  flushSync();
});
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
  document.body.innerHTML = '';
});

const settle = async () => {
  await tick();
  await tick();
};
const key = (k: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init });
  document.activeElement?.dispatchEvent(e);
  return e;
};
const button = () => document.querySelector<HTMLButtonElement>('button.trigger')!;
const boxes = () => [...document.querySelectorAll<HTMLInputElement>('.popup input[type=checkbox]')];
const popup = () => document.querySelector('.popup');

describe('APG-08 multiple choice', () => {
  it('names its state: expanded, no menu semantics (the popup is a group of checkboxes, not a menu)', async () => {
    expect(button().getAttribute('aria-expanded')).toBe('false');
    // aria-haspopup="true" means a menu: a screen reader would announce one and expect menu keys.
    expect(button().hasAttribute('aria-haspopup')).toBe(false);
    button().click();
    await settle();
    expect(button().getAttribute('aria-expanded')).toBe('true');
    expect(popup()?.getAttribute('role')).toBe('group');
    expect(popup()?.getAttribute('aria-label')).toBe('Areas');
  });

  it('ArrowDown on the button opens the popup and puts the focus on the first option', async () => {
    button().focus();
    key('ArrowDown');
    await settle();
    expect(popup()).not.toBeNull();
    expect(document.activeElement).toBe(boxes()[0]);
  });

  it('the arrows walk the options, Home and End go to the ends; the list does not wrap past the ends', async () => {
    button().focus();
    key('ArrowDown');
    await settle();
    key('ArrowDown');
    expect(document.activeElement).toBe(boxes()[1]);
    key('ArrowUp');
    key('ArrowUp');
    expect(document.activeElement).toBe(boxes()[0]);
    key('End');
    expect(document.activeElement).toBe(boxes()[11]);
    key('ArrowDown');
    expect(document.activeElement).toBe(boxes()[11]);
    key('Home');
    expect(document.activeElement).toBe(boxes()[0]);
  });

  it('Down in the search field goes to the first option; Home and End there stay text keys', async () => {
    button().click();
    await settle();
    const search = document.querySelector<HTMLInputElement>('.popup input[type=search]')!;
    search.focus();
    const end = key('End');
    expect(end.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(search);
    key('ArrowDown');
    expect(document.activeElement).toBe(boxes()[0]);
  });

  it('Space ticks the option under the focus (a checkbox does it itself: its click)', async () => {
    button().click();
    await settle();
    boxes()[2]!.focus();
    boxes()[2]!.click();
    await settle();
    expect(selected).toEqual(['v2']);
  });

  it('Escape closes the popup and returns the focus to the button; the choice stays', async () => {
    button().click();
    await settle();
    boxes()[3]!.focus();
    boxes()[3]!.click();
    key('Escape');
    await settle();
    expect(popup()).toBeNull();
    expect(document.activeElement).toBe(button());
    expect(selected).toEqual(['v3']);
  });

  it('Tab out of the popup closes it and is not taken', async () => {
    button().click();
    await settle();
    boxes()[0]!.focus();
    const e = key('Tab');
    expect(e.defaultPrevented).toBe(false);
    document.getElementById('before')!.focus(); // where the browser takes the focus
    await settle();
    expect(popup()).toBeNull();
  });
});
