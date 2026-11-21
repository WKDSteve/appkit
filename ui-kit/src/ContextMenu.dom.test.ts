// @vitest-environment jsdom
// The context menu in a real DOM: a right click on a target opens what its providers say, the keys walk and choose,
// Escape gives the focus back, the browser's menu is left alone without entries and in text fields.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ContextMenu from './ContextMenu.svelte';
import { contextTarget, ContextRegistry } from './contextmenu';

let app: ReturnType<typeof mount> | undefined;
let off: Array<() => void> = [];
const registry = new ContextRegistry();
const chosen = vi.fn();

beforeEach(() => {
  document.body.innerHTML = `
    <section id="view"><button id="card">Card</button><input id="field" /><select id="pick"><option>a</option></select><input id="box" type="checkbox" /></section>
    <p id="plain">plain</p>`;
  const view = document.getElementById('view') as HTMLElement;
  const card = document.getElementById('card') as HTMLElement;
  contextTarget(view, { kind: 'view', id: 'board' });
  contextTarget(card, { kind: 'tile', id: 't1' });
  off = [
    registry.provide('tile', (t) => [
      { kind: 'action', label: 'Open', onselect: () => chosen('open', t.id) },
      { kind: 'action', label: 'Delete', disabled: true, onselect: () => chosen('delete') },
    ]),
    registry.provide('view', () => [{ kind: 'section', label: 'View' }, { kind: 'check', label: 'Compact', checked: true, onselect: () => chosen('compact') }]),
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

const menu = () => document.querySelector('[role="menu"]');
const names = () => [...document.querySelectorAll('[role="menu"] .cm-item')].map((b) => b.textContent?.replace(/[●✓]/g, '').trim());
const rightClick = (el: Element) => {
  const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 20, clientY: 30 });
  el.dispatchEvent(e);
  return e;
};
const key = (k: string, init: KeyboardEventInit = {}) =>
  document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }));
const settle = async () => {
  await tick();
  await tick();
  await tick();
};

describe('ContextMenu', () => {
  it('opens on a right click with the entries of the target and of the view around it, and takes over the click', async () => {
    const e = rightClick(document.getElementById('card')!);
    await settle();
    expect(e.defaultPrevented).toBe(true);
    expect(menu()?.getAttribute('aria-label')).toBe('Context menu');
    expect(names()).toEqual(['Open', 'Delete', 'Compact']);
    // The first entry has the focus; a disabled one is not reached by the keys.
    expect(document.activeElement?.textContent).toContain('Open');
  });

  it('leaves the browser menu alone where nobody offers anything, and in a text field', async () => {
    expect(rightClick(document.getElementById('plain')!).defaultPrevented).toBe(false);
    expect(rightClick(document.getElementById('field')!).defaultPrevented).toBe(false);
    await settle();
    expect(menu()).toBeNull();
  });

  it('walks with the arrow keys, wraps, and chooses with Enter (a click on the entry runs it and closes)', async () => {
    rightClick(document.getElementById('card')!);
    await settle();
    key('ArrowDown');
    expect(document.activeElement?.textContent).toContain('Compact');
    key('ArrowDown');
    expect(document.activeElement?.textContent).toContain('Open');
    key('End');
    expect(document.activeElement?.textContent).toContain('Compact');
    (document.activeElement as HTMLButtonElement).click();
    await settle();
    expect(chosen).toHaveBeenCalledWith('compact');
    expect(menu()).toBeNull();
  });

  it('goes to the entry that a typed letter names', async () => {
    rightClick(document.getElementById('card')!);
    await settle();
    key('c');
    expect(document.activeElement?.textContent).toContain('Compact');
  });

  it('takes every letter while it is open (a letter that names nothing is no shortcut of the page behind it)', async () => {
    const behind = vi.fn();
    window.addEventListener('keydown', (e) => !e.defaultPrevented && behind(e.key));
    rightClick(document.getElementById('card')!);
    await settle();
    key('x');
    key('o');
    expect(behind).not.toHaveBeenCalled();
    // A chord with Ctrl (the palette) is not taken.
    key('k', { ctrlKey: true });
    expect(behind).toHaveBeenCalledWith('k');
  });

  it('hands the target to the entry that was chosen', async () => {
    rightClick(document.getElementById('card')!);
    await settle();
    (document.querySelector('.cm-item') as HTMLButtonElement).click();
    expect(chosen).toHaveBeenCalledWith('open', 't1');
  });

  it('closes with Escape and gives the focus back to the target', async () => {
    const card = document.getElementById('card')!;
    rightClick(card);
    await settle();
    key('Escape');
    await settle();
    expect(menu()).toBeNull();
    expect(document.activeElement).toBe(card);
  });

  it('opens on a choice and a checkbox (they have no text menu), not on a text field', async () => {
    expect(rightClick(document.getElementById('pick')!).defaultPrevented).toBe(true);
    await settle();
    expect(rightClick(document.getElementById('box')!).defaultPrevented).toBe(true);
    expect(rightClick(document.getElementById('field')!).defaultPrevented).toBe(false);
  });

  it('opens with the context key and with Shift+F10 on the focused target', async () => {
    const card = document.getElementById('card') as HTMLButtonElement;
    card.focus();
    card.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }));
    await settle();
    expect(names()).toEqual(['Open', 'Delete', 'Compact']);
    key('Escape');
    await settle();
    card.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }));
    await settle();
    expect(menu()).not.toBeNull();
  });

  it('an element without a target (a component that is one only sometimes) is none', async () => {
    const el = document.getElementById('plain') as HTMLElement;
    const action = contextTarget(el, undefined);
    expect(rightClick(el).defaultPrevented).toBe(false);
    action.update({ kind: 'tile', id: 'x' });
    expect(rightClick(el).defaultPrevented).toBe(true);
    await settle();
    action.update(undefined);
    expect(rightClick(el).defaultPrevented).toBe(false);
  });

  it('stays open on a scroll that leaves its target where it was (one that came late, from before the click), closes when the target moves or goes', async () => {
    const card = document.getElementById('card')!;
    const scroll = async () => {
      document.getElementById('view')!.dispatchEvent(new Event('scroll'));
      await settle();
    };
    rightClick(card);
    await settle();
    await scroll();
    expect(menu()).not.toBeNull();
    vi.spyOn(card, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, -40, 50, 20));
    await scroll();
    expect(menu()).toBeNull();

    vi.mocked(card.getBoundingClientRect).mockRestore();
    rightClick(card);
    await settle();
    card.remove();
    await scroll();
    expect(menu()).toBeNull();
  });

  it('opens inside the open dialog its target is in, so a modal dialog does not cover it (the page behind is inert)', async () => {
    const dialog = document.createElement('dialog');
    dialog.innerHTML = '<button id="chip">Tag</button>';
    document.body.append(dialog);
    dialog.setAttribute('open', '');
    const chip = document.getElementById('chip') as HTMLElement;
    contextTarget(chip, { kind: 'tile', id: 't2' });
    rightClick(chip);
    await settle();
    expect(menu()?.parentElement).toBe(dialog);
    // It still works there: the first entry has the focus, a choice runs and closes it.
    expect(document.activeElement?.textContent).toContain('Open');
    (document.activeElement as HTMLButtonElement).click();
    await settle();
    expect(chosen).toHaveBeenCalledWith('open', 't2');
    expect(menu()).toBeNull();
    // A target outside any dialog opens the menu where it always opened.
    rightClick(document.getElementById('card')!);
    await settle();
    expect(menu()?.parentElement).not.toBe(dialog);
    expect(dialog.contains(menu())).toBe(false);
  });

  it('closes when the dialog it lives in closes', async () => {
    const dialog = document.createElement('dialog');
    dialog.innerHTML = '<button id="chip3">Tag</button>';
    document.body.append(dialog);
    dialog.setAttribute('open', '');
    const chip = document.getElementById('chip3') as HTMLElement;
    contextTarget(chip, { kind: 'tile', id: 't4' });
    rightClick(chip);
    await settle();
    expect(menu()).not.toBeNull();
    dialog.removeAttribute('open');
    dialog.dispatchEvent(new Event('close'));
    await settle();
    expect(menu()).toBeNull();
  });

  it('opens in the page when the dialog of its target is closed', async () => {
    const dialog = document.createElement('dialog');
    dialog.innerHTML = '<button id="chip2">Tag</button>';
    document.body.append(dialog);
    const chip = document.getElementById('chip2') as HTMLElement;
    contextTarget(chip, { kind: 'tile', id: 't3' });
    chip.focus();
    chip.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }));
    await settle();
    expect(dialog.contains(menu())).toBe(false);
  });

  it('closes on a press beside it', async () => {
    rightClick(document.getElementById('card')!);
    await settle();
    document.getElementById('plain')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    await settle();
    expect(menu()).toBeNull();
  });

  it('lays a submenu open in its list, under a heading of its label (no submenu opens here)', async () => {
    const extra = registry.provide('tile', () => [
      { kind: 'submenu', label: 'Move to', items: [{ kind: 'action', label: 'Inbox', onselect: () => chosen('inbox') }] },
    ]);
    rightClick(document.getElementById('card')!);
    await settle();
    extra();
    expect(names()).toEqual(['Open', 'Delete', 'Inbox', 'Compact']);
    expect([...document.querySelectorAll('[role="menu"] .cm-section')].map((p) => p.textContent)).toContain('Move to');
    expect(document.querySelector('[role="menu"] [aria-haspopup]')).toBeNull();
  });
});
