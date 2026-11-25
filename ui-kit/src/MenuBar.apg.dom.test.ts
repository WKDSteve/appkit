// @vitest-environment jsdom
// APG menubar pattern (WCAG 2.1.1, 2.4.3, 4.1.2) for the menu bar: the key sequence of the pattern.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import MenuBar from './MenuBar.svelte';
import type { Menu } from './menu';

const noop = () => {};
const menus: Menu[] = [
  { id: 'file', label: 'File', items: [{ kind: 'action', label: 'New', onselect: noop }, { kind: 'action', label: 'Quit', onselect: noop }] },
  { id: 'edit', label: 'Edit', items: [{ kind: 'action', label: 'Copy', onselect: noop }, { kind: 'action', label: 'Paste', onselect: noop }] },
  { id: 'view', label: 'View', items: [{ kind: 'action', label: 'Zoom', onselect: noop }] },
];

let app: ReturnType<typeof mount> | undefined;
beforeEach(() => {
  document.body.innerHTML = '<button id="before">Before</button>';
  app = mount(MenuBar, { target: document.body, props: { menus, label: 'Main' } });
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
const trigger = (id: string) => document.querySelector<HTMLButtonElement>(`button.mb-trigger[data-for="${id}"]`)!;
const popup = (id: string) => document.querySelector(`[data-menu="${id}"]`);
const here = () => document.activeElement?.textContent?.trim();

describe('APG menubar pattern', () => {
  it('has the roles of the pattern and one tab stop for the whole bar', () => {
    const bar = document.querySelector('[role="menubar"]')!;
    expect(bar.getAttribute('aria-label')).toBe('Main');
    const triggers = [...bar.querySelectorAll<HTMLButtonElement>('button.mb-trigger')];
    expect(triggers.every((t) => t.getAttribute('role') === 'menuitem' && t.getAttribute('aria-haspopup') === 'menu')).toBe(true);
    expect(triggers.map((t) => t.tabIndex)).toEqual([0, -1, -1]);
    expect(triggers.every((t) => t.getAttribute('aria-expanded') === 'false')).toBe(true);
  });

  it('Left/Right/Home/End walk the titles of the closed bar and wrap, without opening a menu', () => {
    trigger('file').focus();
    key('ArrowRight');
    expect(document.activeElement).toBe(trigger('edit'));
    key('End');
    expect(document.activeElement).toBe(trigger('view'));
    key('ArrowRight');
    expect(document.activeElement).toBe(trigger('file'));
    key('ArrowLeft');
    expect(document.activeElement).toBe(trigger('view'));
    key('Home');
    expect(document.activeElement).toBe(trigger('file'));
    expect(document.querySelector('[role="menu"]')).toBeNull();
    // The tab stop follows the focus.
    expect(trigger('file').tabIndex).toBe(0);
    expect(trigger('view').tabIndex).toBe(-1);
  });

  it('a typed letter goes to the next title that starts with it', async () => {
    trigger('file').focus();
    key('v');
    expect(document.activeElement).toBe(trigger('view'));
    await new Promise((r) => setTimeout(r, 850)); // the word is over after TYPE_AHEAD_MS
    key('e');
    expect(document.activeElement).toBe(trigger('edit'));
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it('Enter, Space and Down open the menu on its first item; Up opens it on its last', async () => {
    for (const k of ['Enter', ' ', 'ArrowDown']) {
      trigger('edit').focus();
      key(k);
      await settle();
      expect(here()).toBe('Copy');
      key('Escape');
      await settle();
    }
    trigger('edit').focus();
    key('ArrowUp');
    await settle();
    expect(here()).toBe('Paste');
  });

  it('Down/Up/Home/End walk the items and wrap; Escape closes onto the title', async () => {
    trigger('file').focus();
    key('ArrowDown');
    await settle();
    key('ArrowDown');
    expect(here()).toBe('Quit');
    key('ArrowDown');
    expect(here()).toBe('New');
    key('ArrowUp');
    expect(here()).toBe('Quit');
    key('Home');
    expect(here()).toBe('New');
    key('End');
    expect(here()).toBe('Quit');
    key('Escape');
    await settle();
    expect(popup('file')).toBeNull();
    expect(document.activeElement).toBe(trigger('file'));
    expect(trigger('file').getAttribute('aria-expanded')).toBe('false');
  });

  it('Left/Right inside a menu open the neighbour menu on its first item, and wrap', async () => {
    trigger('file').focus();
    key('ArrowDown');
    await settle();
    key('ArrowRight');
    await settle();
    expect(popup('file')).toBeNull();
    expect(here()).toBe('Copy');
    key('ArrowLeft');
    await settle();
    key('ArrowLeft'); // from the first menu back to the last
    await settle();
    expect(here()).toBe('Zoom');
  });

  it('with a menu open, Left/Right/Home/End on a title open that menu (the pattern moves the open menu along)', async () => {
    trigger('file').click();
    await settle();
    trigger('file').focus();
    key('ArrowRight');
    await settle();
    expect(popup('edit')).not.toBeNull();
    expect(popup('file')).toBeNull();
    key('End');
    await settle();
    expect(popup('view')).not.toBeNull();
  });

  it('a typed letter inside a menu goes to the next item that starts with it', async () => {
    trigger('edit').focus();
    key('ArrowDown');
    await settle();
    key('p');
    expect(here()).toBe('Paste');
  });

  it('Tab closes the open menu and is not taken', async () => {
    trigger('file').focus();
    key('ArrowDown');
    await settle();
    const e = key('Tab');
    await settle();
    expect(e.defaultPrevented).toBe(false);
    expect(popup('file')).toBeNull();
  });
});
