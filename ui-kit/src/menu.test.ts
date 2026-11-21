import { describe, expect, it } from 'vitest';
import {
  ariaShortcut,
  letterOfCode,
  menuForMnemonic,
  popupShift,
  canAnchorPopover,
  anchorName,
  flatItems,
  flyoutPlace,
  inCorridor,
  splitMnemonic,
  subItems,
  submenuCurrent,
  submenuUsable,
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

describe('canAnchorPopover', () => {
  const all = () => true;
  it('needs the Popover API and anchor positioning together', () => {
    expect(canAnchorPopover({ showPopover: () => {}, supports: all })).toBe(true);
    expect(canAnchorPopover({ supports: all })).toBe(false);
    expect(canAnchorPopover({ showPopover: () => {}, supports: (c) => !c.startsWith('position-area') })).toBe(false);
    expect(canAnchorPopover({ showPopover: () => {} })).toBe(false);
  });
  it('is false when the check itself throws', () => {
    expect(
      canAnchorPopover({
        showPopover: () => {},
        supports: () => {
          throw new Error('no CSS');
        },
      }),
    ).toBe(false);
  });
});

describe('anchorName', () => {
  it('is a dashed ident, whatever the id holds', () => {
    expect(anchorName('mb-x1', 'file')).toBe('--mb-x1-file');
    expect(anchorName('mb', 'a b/c:d')).toBe('--mb-a_b_c_d');
  });
});

describe('submenu items', () => {
  const radio = (label: string, checked: boolean): MenuItem => ({ kind: 'radio', label, checked, onselect: () => {} });
  const sub = (label: string, items: MenuItem[]): MenuItem & { kind: 'submenu' } => ({ kind: 'submenu', label, items });

  it('lays a submenu inside a submenu open under its label: one level, nothing lost', () => {
    const inner = sub('Inner', [act('Deep'), sub('Deeper', [act('Deepest')])]);
    const items = subItems(sub('Outer', [act('A'), inner]));
    expect(items.map((i) => `${i.kind}:${'label' in i ? i.label : ''}`)).toEqual([
      'action:A',
      'section:Inner',
      'action:Deep',
      'section:Deeper',
      'action:Deepest',
    ]);
    expect(items.some((i) => (i as MenuItem).kind === 'submenu')).toBe(false);
  });

  it('lays every submenu of a list open for a list that opens none (the context menu)', () => {
    const items = flatItems([act('A'), sub('Views', [act('Plan')]), act('B')]);
    expect(labelsOf(items)).toEqual(['A', 'Views', 'Plan', 'B']);
    expect(items[1]?.kind).toBe('section');
  });

  it('names the checked radio item inside, at any depth, or null', () => {
    expect(submenuCurrent(sub('Plans', [radio('Map', false), radio('Paths', true)]))).toBe('Paths');
    expect(submenuCurrent(sub('Plans', [sub('More', [radio('Map', true)])]))).toBe('Map');
    expect(submenuCurrent(sub('Plans', [radio('Map', false), act('Paths')]))).toBeNull();
  });

  it('is usable with an item that can be chosen, not when disabled or without one', () => {
    expect(submenuUsable(sub('Plans', [act('Map')]))).toBe(true);
    expect(submenuUsable({ ...sub('Plans', [act('Map')]), disabled: true })).toBe(false);
    expect(submenuUsable(sub('Plans', [{ kind: 'info', label: 'none yet' }]))).toBe(false);
    expect(submenuUsable(sub('Plans', [{ kind: 'action', label: 'Off', onselect: () => {}, disabled: true }]))).toBe(
      false,
    );
  });
});

describe('flyoutPlace', () => {
  const view = { width: 1000, height: 600 };
  const size = { width: 200, height: 100 };
  const parent = (left: number, right: number, top = 100) => ({ left, right, top, bottom: top + 24 });

  it('goes to the right of its item where it fits, its first item level with the item', () => {
    expect(flyoutPlace(parent(100, 300), size, view, 5)).toEqual({ left: 300, top: 95, side: 'right' });
  });
  it('goes to the left where the right has no room', () => {
    expect(flyoutPlace(parent(700, 900), size, view)).toEqual({ left: 500, top: 100, side: 'left' });
  });
  it('takes the side with more room when neither fits, and stays in the window', () => {
    const wide = { width: 400, height: 100 };
    const narrowView = { width: 500, height: 600 };
    const at = flyoutPlace(parent(150, 350), wide, narrowView);
    expect(at.side).toBe('right');
    expect(at.left).toBe(500 - 8 - 400);
    const left = flyoutPlace(parent(200, 450), wide, narrowView);
    expect(left.side).toBe('left');
    expect(left.left).toBe(8);
  });
  it('moves up at the bottom of the window, and keeps the margin at the top', () => {
    expect(flyoutPlace(parent(100, 300, 550), size, view).top).toBe(600 - 8 - 100);
    expect(flyoutPlace(parent(100, 300, 2), size, view, 5).top).toBe(8);
  });
});

describe('inCorridor', () => {
  const target = { left: 300, right: 500, top: 100, bottom: 300 };
  it('is true on the way from the item to the near edge of the submenu', () => {
    expect(inCorridor({ x: 200, y: 110 }, { x: 260, y: 150 }, target, 'right')).toBe(true);
    expect(inCorridor({ x: 200, y: 110 }, { x: 280, y: 250 }, target, 'right')).toBe(true);
  });
  it('is false straight down to a neighbour, or away from the submenu', () => {
    expect(inCorridor({ x: 200, y: 110 }, { x: 202, y: 140 }, target, 'right')).toBe(false);
    expect(inCorridor({ x: 200, y: 110 }, { x: 150, y: 130 }, target, 'right')).toBe(false);
  });
  it('keeps its tip where the pointer left, with a few pixels of room around it', () => {
    expect(inCorridor({ x: 200, y: 110 }, { x: 197, y: 110 }, target, 'right')).toBe(true);
    expect(inCorridor({ x: 200, y: 110 }, { x: 190, y: 110 }, target, 'right')).toBe(false);
    expect(inCorridor({ x: 200, y: 110 }, { x: 197, y: 110 }, target, 'right', 0)).toBe(false);
  });
  it('looks to the far edge of a submenu on the left', () => {
    const left = { left: 0, right: 150, top: 100, bottom: 300 };
    expect(inCorridor({ x: 250, y: 110 }, { x: 200, y: 160 }, left, 'left')).toBe(true);
    expect(inCorridor({ x: 250, y: 110 }, { x: 300, y: 160 }, left, 'left')).toBe(false);
  });
});
