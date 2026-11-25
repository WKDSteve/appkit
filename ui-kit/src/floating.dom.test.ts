// @vitest-environment jsdom
// The action of an app in a dialog of the kit: it is given the dialog and its words, and keeps its places under the key
// of the app.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import DocsDialog from './DocsDialog.svelte';
import { floatingIn, type FloatAction, type FloatingParams } from './floating';

const labels = { title: 'Docs', label: 'Search', placeholder: 'Type', none: 'None', list: 'List', close: 'Close' };

describe('a dialog of the kit', () => {
  let app: ReturnType<typeof mount> | undefined;
  afterEach(() => {
    if (app) void unmount(app);
    app = undefined;
    document.body.innerHTML = '';
  });

  it('stays where the browser puts it without an action of the app', () => {
    app = mount(DocsDialog, { target: document.body, props: { open: false, docs: [], onclose: () => {}, labels } });
    flushSync();
    expect(document.querySelector('dialog')?.classList.contains('floating')).toBe(false);
  });

  it('takes the action of the app, with its words', () => {
    const seen: [HTMLDialogElement, FloatingParams][] = [];
    const float: FloatAction = (node, params) => {
      seen.push([node, params]);
      return { update() {}, destroy() {} };
    };
    const floatParams = { kind: 'docs', moveLabel: 'Move', moveHint: 'Drag' };
    app = mount(DocsDialog, {
      target: document.body,
      props: { open: false, docs: [], onclose: () => {}, labels, float, floatParams },
    });
    flushSync();
    expect(seen).toEqual([[document.querySelector('dialog'), floatParams]]);
  });
});

describe('floatingIn', () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.stubGlobal('CSS', { escape: (s: string) => s });
  });
  afterEach(() => vi.unstubAllGlobals());

  it('a float that closes during a drag keeps the place it had', () => {
    const dialog = document.createElement('dialog');
    document.body.append(dialog);
    const action = floatingIn('an-app.floats')(dialog, { kind: 'docs', moveLabel: 'Move', moveHint: 'Drag' });
    dialog.setAttribute('open', '');
    const at = (x: number) => ({ left: x, top: 10, width: 100, height: 100 }) as DOMRect;
    dialog.getBoundingClientRect = () => at(50);
    const handle = dialog.querySelector('.float-move') as HTMLElement;
    handle.setPointerCapture = () => {};
    const pointer = (type: string, clientX: number) =>
      Object.assign(new MouseEvent(type, { bubbles: true, button: 0, clientX, clientY: 10 }), { pointerId: 1 });
    handle.dispatchEvent(pointer('pointerdown', 60));
    // Closed: what it measures now is nothing.
    dialog.removeAttribute('open');
    dialog.getBoundingClientRect = () => at(0);
    handle.dispatchEvent(pointer('pointerup', 60));
    expect(window.localStorage.getItem('an-app.floats')).toBeNull();
    action.destroy();
    dialog.remove();
  });

  it('keeps the place under the key of the app', () => {
    const dialog = document.createElement('dialog');
    document.body.append(dialog);
    const action = floatingIn('an-app.floats')(dialog, { kind: 'docs', moveLabel: 'Move', moveHint: 'Drag' });
    const handle = dialog.querySelector('.float-move');
    expect(handle?.getAttribute('aria-label')).toBe('Move');
    dialog.setAttribute('open', '');
    handle?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    expect(Object.keys(JSON.parse(window.localStorage.getItem('an-app.floats') ?? '{}'))).toEqual(['docs']);
    action.destroy();
    dialog.remove();
  });

  it('moves and sizes with the arrow keys, and puts back with Home (the alternative to dragging)', () => {
    const dialog = document.createElement('dialog');
    document.body.append(dialog);
    const action = floatingIn('an-app.floats')(dialog, { kind: 'docs', moveLabel: 'Move', moveHint: 'Arrows move' });
    dialog.setAttribute('open', '');
    // The box follows what the action wrote, as a browser lays it out.
    dialog.getBoundingClientRect = () =>
      ({
        left: parseFloat(dialog.style.left || '100'),
        top: parseFloat(dialog.style.top || '100'),
        width: parseFloat(dialog.style.width || '200'),
        height: parseFloat(dialog.style.height || '200'),
      }) as DOMRect;
    const handle = dialog.querySelector('.float-move') as HTMLElement;
    const press = (key: string, shiftKey = false) =>
      handle.dispatchEvent(new KeyboardEvent('keydown', { key, shiftKey, bubbles: true, cancelable: true }));
    press('ArrowRight');
    const moved = parseFloat(dialog.style.left);
    expect(moved).toBeGreaterThan(100);
    expect(dialog.style.width).toBe('');
    press('ArrowRight', true);
    expect(parseFloat(dialog.style.width)).toBeGreaterThan(200);
    press('Home');
    expect(dialog.style.left).toBe('');
    expect(window.localStorage.getItem('an-app.floats')).toBe('{}');
    action.destroy();
    dialog.remove();
  });

  it('names what the handle does for a screen reader, not only in the hint popup', () => {
    const dialog = document.createElement('dialog');
    document.body.append(dialog);
    const action = floatingIn('an-app.floats')(dialog, { kind: 'docs', moveLabel: 'Move', moveHint: 'Arrows move' });
    const handle = dialog.querySelector('.float-move') as HTMLElement;
    const described = document.getElementById(handle.getAttribute('aria-describedby') ?? '');
    expect(described?.textContent).toBe('Arrows move');
    action.update({ kind: 'docs', moveLabel: 'Verschieben', moveHint: 'Pfeiltasten' });
    expect(described?.textContent).toBe('Pfeiltasten');
    action.destroy();
    expect(dialog.querySelector('.sr-only')).toBeNull();
    dialog.remove();
  });

  it('draws the handle first in the order of the focus', () => {
    const dialog = document.createElement('dialog');
    const input = document.createElement('input');
    dialog.append(input);
    document.body.append(dialog);
    const action = floatingIn('an-app.floats')(dialog, { kind: 'docs', moveLabel: 'Move', moveHint: 'Arrows move' });
    expect(dialog.firstElementChild?.classList.contains('float-move')).toBe(true);
    action.destroy();
    dialog.remove();
  });
});
