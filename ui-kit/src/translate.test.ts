import { describe, expect, it } from 'vitest';
import { unprotected } from './checks/translate';
import blocks from './Blocks.svelte?raw';
import conflict from './ConflictCenter.svelte?raw';
import contextMenu from './ContextMenu.svelte?raw';
import inlines from './Inlines.svelte?raw';
import menuBar from './MenuBar.svelte?raw';

// HTML-05: code, commands, identifiers and the shortcuts of a menu are not for the translation of the browser.
// A `<code>` straight inside a `<pre translate="no">` is covered by that `<pre>`.
const SOURCES: Record<string, string> = { Blocks: blocks, ConflictCenter: conflict, ContextMenu: contextMenu, Inlines: inlines, MenuBar: menuBar };

describe('translate="no" in the kit (HTML-05)', () => {
  for (const [name, source] of Object.entries(SOURCES)) {
    it(`${name}: every code, pre, kbd and shortcut says translate="no"`, () => {
      expect(unprotected(source)).toEqual([]);
    });
  }

  it('the check itself sees a bare code', () => {
    expect(unprotected('<p><code>x</code></p>')).toEqual(['<code>']);
  });

  it('finds an unprotected element, and not a protected one', () => {
    const src =
      '<code>a</code><code translate="no">b</code><pre translate="no"><code>c</code></pre><pre><code>d</code></pre>' +
      '<kbd>j</kbd><span class="id">x</span><span class="id" translate="no">y</span><pre translate="yes">prose</pre>' +
      '<span class="cm-shortcut">k</span><span class="mb-shortcut" translate="no">l</span>';
    expect(unprotected(src)).toEqual(['<code>', '<pre>', '<code>', '<kbd>', '<span class="id">', '<span class="cm-shortcut">']);
  });
});
