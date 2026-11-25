// @vitest-environment jsdom
// APG menu pattern (WCAG 2.1.1, 2.4.3, 4.1.2) for the context menu: the key sequence of the pattern.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ContextMenu from './ContextMenu.svelte';
import { contextTarget, ContextRegistry } from './contextmenu';

let app: ReturnType<typeof mount> | undefined;
let off: Array<() => void> = [];
const chosen = vi.fn();

beforeEach(() => {
  document.body.innerHTML = '<button id="card">Card</button><button id="after">After</button>';
  const registry = new ContextRegistry();
  contextTarget(document.getElementById('card')!, { kind: 'tile', id: 't1' });
  off = [
    registry.provide('tile', () => [
      { kind: 'action', label: 'Open', onselect: () => chosen('open') },
      { kind: 'action', label: 'Archive', disabled: true, onselect: () => chosen('archive') },
      { kind: 'check', label: 'Pinned', checked: true, onselect: () => chosen('pin') },
      { kind: 'radio', label: 'Sorted', checked: false, onselect: () => chosen('sort') },
      { kind: 'action', label: 'Delete', onselect: () => chosen('delete') },
    ]),
  ];
  app = mount(ContextMenu, { target: document.body, props: { registry, label: 'Context menu' } });
  flushSync();
  chosen.mockReset();
});
afterEach(async () => {
  for (const f of off) f();
  if (app) await unmount(app);
  app = undefined;
  document.body.innerHTML = '';
});

const settle = async () => {
  await tick();
  await tick();
  await tick();
};
const key = (k: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init });
  document.activeElement?.dispatchEvent(e);
  return e;
};
const here = () => document.activeElement?.textContent?.replace(/[●✓]/g, '').trim();
const openByKey = async () => {
  const card = document.getElementById('card') as HTMLButtonElement;
  card.focus();
  card.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }));
  await settle();
};

describe('APG menu pattern: context menu', () => {
  it('has the roles of the pattern: a menu with named items, checked states, one focus stop each', async () => {
    await openByKey();
    const menu = document.querySelector('[role="menu"]')!;
    expect(menu.getAttribute('aria-label')).toBe('Context menu');
    const items = [...menu.querySelectorAll('button')];
    expect(items.map((i) => i.getAttribute('role'))).toEqual([
      'menuitem',
      'menuitem',
      'menuitemcheckbox',
      'menuitemradio',
      'menuitem',
    ]);
    expect(items[2]!.getAttribute('aria-checked')).toBe('true');
    expect(items[3]!.getAttribute('aria-checked')).toBe('false');
    // Roving focus: no item is a tab stop, the focus is set by script.
    expect(items.every((i) => i.tabIndex === -1)).toBe(true);
  });

  it('opens by the keyboard on the first item; Down/Up walk, skip the disabled item and wrap both ways', async () => {
    await openByKey();
    expect(here()).toBe('Open');
    key('ArrowDown');
    expect(here()).toBe('Pinned'); // "Archive" is disabled
    key('ArrowUp');
    expect(here()).toBe('Open');
    key('ArrowUp'); // wraps from the first to the last
    expect(here()).toBe('Delete');
    key('ArrowDown'); // and from the last to the first
    expect(here()).toBe('Open');
  });

  it('Home and End go to the first and last enabled item', async () => {
    await openByKey();
    key('End');
    expect(here()).toBe('Delete');
    key('Home');
    expect(here()).toBe('Open');
  });

  it('type-ahead goes to the next item that starts with the letter, and a longer word stays on its item', async () => {
    await openByKey();
    key('d');
    expect(here()).toBe('Delete');
    key('o');
    key('p');
    expect(here()).toBe('Delete'); // "dop" names nothing: the focus stays where it is
    await new Promise((r) => setTimeout(r, 850)); // the word is over after TYPE_AHEAD_MS
    key('s');
    expect(here()).toBe('Sorted');
  });

  it('Enter and Space on an item run it and close the menu with the focus back on the trigger', async () => {
    await openByKey();
    key('ArrowDown'); // Pinned
    // A button turns Enter and Space into a click (jsdom does not): the click is what the keys do.
    (document.activeElement as HTMLButtonElement).click();
    await settle();
    expect(chosen).toHaveBeenCalledWith('pin');
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(document.getElementById('card'));
  });

  it('Escape closes and returns the focus to the trigger', async () => {
    await openByKey();
    key('End');
    key('Escape');
    await settle();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(document.getElementById('card'));
  });

  it('Tab closes the menu and is not taken (the focus moves on in the page, no trap)', async () => {
    await openByKey();
    const e = key('Tab');
    await settle();
    expect(e.defaultPrevented).toBe(false);
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });
});
