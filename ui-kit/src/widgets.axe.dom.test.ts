// @vitest-environment jsdom
// The four widgets with axe (roles, names, states, the relations between them), in the light and in the dark theme. jsdom
// has no layout and no colours, so axe cannot measure contrast here: the contrast of the colours is held in
// tokens.test.ts from the values of tokens.css, and what these runs hold is that the markup is the same, and sound, in
// both themes.
import axe from 'axe-core';
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ContextMenu from './ContextMenu.svelte';
import MenuBar from './MenuBar.svelte';
import MultiSelect from './MultiSelect.svelte';
import { contextTarget, ContextRegistry } from './contextmenu';
import type { Menu } from './menu';
import { installTooltips, TIP_ID } from './tooltip';

const noop = () => {};
const settle = async () => {
  await tick();
  await tick();
};

let app: ReturnType<typeof mount> | undefined;
let uninstall: (() => void) | undefined;
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
  uninstall?.();
  uninstall = undefined;
  document.body.innerHTML = '';
  document.documentElement.removeAttribute('data-theme');
  vi.useRealTimers();
});

/** The violations of axe on the page as it is now (contrast is left to tokens.test.ts: it cannot be measured here). */
async function violations(theme: 'light' | 'dark'): Promise<string[]> {
  document.documentElement.setAttribute('data-theme', theme);
  const result = await axe.run(document.body, {
    rules: { 'color-contrast': { enabled: false }, region: { enabled: false } },
  });
  return result.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`);
}

const themes = ['light', 'dark'] as const;

describe('axe on the widgets', () => {
  const menus: Menu[] = [
    { id: 'file', label: 'File', items: [{ kind: 'action', label: 'New', onselect: noop }, { kind: 'action', label: 'Quit', onselect: noop }] },
    { id: 'edit', label: 'Edit', items: [{ kind: 'action', label: 'Copy', onselect: noop }] },
  ];

  it('MenuBar, with a menu open', async () => {
    app = mount(MenuBar, { target: document.body, props: { menus, label: 'Main' } });
    flushSync();
    const first = document.querySelector<HTMLElement>('[role="menuitem"]');
    first?.focus();
    first?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    await settle();
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    for (const theme of themes) expect(await violations(theme), theme).toEqual([]);
  });

  it('MultiSelect, with the group open', async () => {
    const options = Array.from({ length: 12 }, (_, i) => ({ value: `v${i}`, label: `Option ${i}` }));
    app = mount(MultiSelect, {
      target: document.body,
      props: { label: 'Areas', options, selected: ['v1'], onchange: noop, resetLabel: 'Reset', searchLabel: 'Search' },
    });
    flushSync();
    document.querySelector<HTMLButtonElement>('button.trigger')?.click();
    await settle();
    expect(document.querySelector('.popup')).not.toBeNull();
    for (const theme of themes) expect(await violations(theme), theme).toEqual([]);
  });

  it('ContextMenu, open', async () => {
    document.body.innerHTML = '<button id="card">Card</button>';
    const registry = new ContextRegistry();
    contextTarget(document.getElementById('card')!, { kind: 'tile', id: 't1' });
    registry.provide('tile', () => [
      { kind: 'action', label: 'Open', onselect: noop },
      { kind: 'check', label: 'Pinned', checked: true, onselect: noop },
      { kind: 'radio', label: 'Sorted', checked: false, onselect: noop },
    ]);
    app = mount(ContextMenu, { target: document.body, props: { registry, label: 'Context menu' } });
    flushSync();
    const card = document.getElementById('card') as HTMLButtonElement;
    card.focus();
    card.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }));
    await settle();
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    for (const theme of themes) expect(await violations(theme), theme).toEqual([]);
  });

  it('Tooltip, shown on a button', async () => {
    vi.useFakeTimers();
    document.body.innerHTML = '<button id="a" data-hint="A hint">A</button>';
    uninstall = installTooltips(document, window, { regionLabel: () => 'Hints' });
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    document.getElementById('a')?.focus();
    vi.advanceTimersByTime(2000);
    expect((document.getElementById(TIP_ID) as HTMLElement).hidden).toBe(false);
    vi.useRealTimers();
    for (const theme of themes) expect(await violations(theme), theme).toEqual([]);
  });

  it('finds a fault (so that an empty list means something): a button without a name', async () => {
    document.body.innerHTML = '<button></button>';
    expect(await violations('dark')).not.toEqual([]);
  });
});
