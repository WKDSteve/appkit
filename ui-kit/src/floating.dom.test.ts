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
