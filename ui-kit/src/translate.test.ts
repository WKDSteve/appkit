import { describe, expect, it } from 'vitest';
import blocks from './Blocks.svelte?raw';
import conflict from './ConflictCenter.svelte?raw';
import contextMenu from './ContextMenu.svelte?raw';
import inlines from './Inlines.svelte?raw';
import menuBar from './MenuBar.svelte?raw';

// HTML-05: code, commands, identifiers and the shortcuts of a menu are not for the translation of the browser.
// A `<code>` straight inside a `<pre translate="no">` is covered by that `<pre>`.
const SOURCES: Record<string, string> = { Blocks: blocks, ConflictCenter: conflict, ContextMenu: contextMenu, Inlines: inlines, MenuBar: menuBar };

function unprotected(source: string): string[] {
  const bad: string[] = [];
  for (const m of source.matchAll(/<(code|pre|kbd)\b[^>]*>/g)) {
    const tag = m[0];
    if (/translate="no"/.test(tag)) continue;
    const before = source.slice(Math.max(0, (m.index ?? 0) - 80), m.index);
    if (tag.startsWith('<code') && /<pre\b[^>]*translate="no"[^>]*>\s*$/.test(before)) continue;
    bad.push(tag);
  }
  for (const m of source.matchAll(/<span class="(?:cm|mb)-shortcut"[^>]*>/g)) {
    if (!/translate="no"/.test(m[0])) bad.push(m[0]);
  }
  return bad;
}

describe('translate="no" in the kit (HTML-05)', () => {
  for (const [name, source] of Object.entries(SOURCES)) {
    it(`${name}: every code, pre, kbd and shortcut says translate="no"`, () => {
      expect(unprotected(source)).toEqual([]);
    });
  }

  it('the check itself sees a bare code', () => {
    expect(unprotected('<p><code>x</code></p>')).toEqual(['<code>']);
  });
});
