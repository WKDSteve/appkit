import { describe, expect, it } from 'vitest';
import {
  ariaShortcut,
  letterOfCode,
  menuForMnemonic,
  popupShift,
  splitMnemonic,
  toBlocks,
  typeAheadIndex,
  type Menu,
  type MenuBlock,
  type MenuItem,
} from './menu';

const act = (label: string): MenuItem => ({ kind: 'action', label, onselect: () => {} });
const itemsOf = (b: MenuBlock | undefined): MenuItem[] => (b?.kind === 'group' ? b.items : []);
const labelsOf = (items: MenuItem[]): string[] => items.map((i) => ('label' in i ? i.label : ''));

describe('toBlocks', () => {
  it('puts the items under a section into a group with its heading, a divider before it', () => {
    const blocks = toBlocks([act('New'), { kind: 'section', label: 'Vault' }, act('Open'), act('Close')]);
    expect(blocks.map((b) => b.kind)).toEqual(['group', 'divider', 'group']);
    expect(blocks[0]).toMatchObject({ heading: null });
    expect(blocks[2]).toMatchObject({ heading: 'Vault' });
    expect(labelsOf(itemsOf(blocks[2]))).toEqual(['Open', 'Close']);
  });
  it('has no divider first, last or doubled, and drops a section without items', () => {
    const blocks = toBlocks([
      { kind: 'separator' },
      { kind: 'section', label: 'Empty' },
      { kind: 'separator' },
      act('A'),
      { kind: 'separator' },
      { kind: 'separator' },
      act('B'),
      { kind: 'section', label: 'Trailing' },
      { kind: 'separator' },
    ]);
    expect(blocks.map((b) => b.kind)).toEqual(['group', 'divider', 'group']);
  });
  it('keeps a note inside its group', () => {
    const blocks = toBlocks([{ kind: 'section', label: 'S' }, { kind: 'info', label: 'n' }, act('A')]);
    expect(blocks).toHaveLength(1);
    expect(itemsOf(blocks[0])).toHaveLength(2);
  });
  it('gives nothing for nothing', () => {
    expect(toBlocks([])).toEqual([]);
  });
});

describe('mnemonics', () => {
  const menus: Menu[] = [
    { id: 'p', label: 'Project', mnemonic: 'P', items: [] },
    { id: 'v', label: 'View', mnemonic: 'v', items: [] },
    { id: 'x', label: 'Plain', items: [] },
  ];
  it('finds the menu of Alt+letter, whatever the case', () => {
    expect(menuForMnemonic(menus, 'p')?.id).toBe('p');
    expect(menuForMnemonic(menus, 'V')?.id).toBe('v');
    expect(menuForMnemonic(menus, 'z')).toBeUndefined();
    expect(menuForMnemonic(menus, 'Escape')).toBeUndefined();
    expect(menuForMnemonic(menus, '')).toBeUndefined();
  });
  it('marks the first such letter of a label, and nothing when it is not there', () => {
    expect(splitMnemonic('Project', 'o')).toEqual({ before: 'Pr', mark: 'o', after: 'ject' });
    expect(splitMnemonic('Project', 'P')).toEqual({ before: '', mark: 'P', after: 'roject' });
    expect(splitMnemonic('Project', 'z')).toEqual({ before: 'Project', mark: '', after: '' });
    expect(splitMnemonic('Project', undefined)).toEqual({ before: 'Project', mark: '', after: '' });
  });
  it('cuts the original label, also where lower-casing changes the length', () => {
    // 'İ'.toLocaleLowerCase() is two code units: an index from the lower-cased text pointed one too far.
    expect(splitMnemonic('İstatistik', 's')).toEqual({ before: 'İ', mark: 's', after: 'tatistik' });
  });
  it('names a physical key by its letter', () => {
    expect(letterOfCode('KeyP')).toBe('p');
    expect(letterOfCode('Digit1')).toBeUndefined();
    expect(letterOfCode('')).toBeUndefined();
  });
});

describe('ariaShortcut', () => {
  it('writes the modifier keys as aria-keyshortcuts knows them', () => {
    expect(ariaShortcut('Ctrl+N')).toBe('Control+N');
    expect(ariaShortcut('Ctrl + Shift + S')).toBe('Control+Shift+S');
    expect(ariaShortcut('Cmd+,')).toBe('Meta+,');
    expect(ariaShortcut('F5')).toBe('F5');
  });
});

describe('typeAheadIndex', () => {
  const labels = ['Open', 'Options', 'Save', 'Save as', 'Quit'];
  it('goes to the next item with that letter and wraps', () => {
    expect(typeAheadIndex(labels, 0, 'o')).toBe(1);
    expect(typeAheadIndex(labels, 1, 'o')).toBe(0);
    expect(typeAheadIndex(labels, 0, 'q')).toBe(4);
  });
  it('cycles on the same letter typed again', () => {
    expect(typeAheadIndex(labels, 2, 'ss')).toBe(3);
    expect(typeAheadIndex(labels, 3, 'sss')).toBe(2);
  });
  it('stays on the item a longer word already names', () => {
    expect(typeAheadIndex(labels, 1, 'op')).toBe(1);
    expect(typeAheadIndex(labels, 0, 'save a')).toBe(3);
  });
  it('is -1 when nothing fits', () => {
    expect(typeAheadIndex(labels, 0, 'z')).toBe(-1);
    expect(typeAheadIndex(labels, 0, '')).toBe(-1);
    expect(typeAheadIndex([], 0, 'a')).toBe(-1);
  });
});

describe('popupShift', () => {
  it('leaves a popup that fits where it is', () => {
    expect(popupShift(10, 200, 375)).toBe(0);
  });
  it('moves one that reaches past the right edge back in, to the margin', () => {
    // View at 173 px, a popup of 13rem: it reached to 410 px in a window 375 px wide.
    expect(popupShift(173, 410, 375)).toBe(375 - 8 - 410);
  });
  it('moves one that starts left of the window to the right', () => {
    expect(popupShift(-30, 150, 375)).toBe(38);
  });
  it('keeps the left edge of one wider than the window at the margin', () => {
    expect(popupShift(100, 500, 320)).toBe(-92);
  });
});
