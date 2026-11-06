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
});
