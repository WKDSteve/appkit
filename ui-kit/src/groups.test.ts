import { describe, expect, it } from 'vitest';
import { cellsOf, countOf, groupsOf, neighbourCell, neighbourGroup, NONE, type GroupingLike } from './groups';

// An item of any app: an id and its values per field; the app reads them (`valuesOf`).
type Item = { id: string; values: Record<string, string[]> };
type Field = GroupingLike & { field: string };
const item = (id: string, values: Record<string, string[]> = {}): Item => ({ id, values });
const read = (it: Item, g: Field): readonly string[] => it.values[g.field] ?? [];
const STATUS: Field = { field: 'status', values: ['doing', 'open', 'parked'] };
const TAGS: Field = { field: 'tags', values: ['ui', 'bug'] };
const SEVERITY: Field = { field: 'severity', values: ['critical', 'recommended'] };

describe('the groups', () => {
  it('has one group for each value the engine names, in its order, and keeps the empty ones as places to drop', () => {
    const groups = groupsOf([item('a', { status: ['open'] }), item('b', { status: ['doing'] }), item('c', { status: ['open'] })], STATUS, read);
    expect(groups.map((g) => [g.value, g.items.map((i) => i.id)])).toEqual([
      ['doing', ['b']],
      ['open', ['a', 'c']],
      ['parked', []],
    ]);
    expect(groups.map(countOf)).toEqual([1, 2, 0]);
  });

  it('keeps the order the items came in, and never sorts them', () => {
    const groups = groupsOf([item('z', { status: ['open'] }), item('a', { status: ['open'] })], STATUS, read);
    expect(groups[1]?.items.map((i) => i.id)).toEqual(['z', 'a']);
  });

  it('puts the items that have none in a last group of their own, and only when there are some', () => {
    expect(groupsOf([item('a', { status: ['open'] })], STATUS, read).some((g) => g.value === NONE)).toBe(false);
    const groups = groupsOf([item('a', { status: ['open'] }), item('b')], STATUS, read);
    expect(groups.at(-1)).toEqual({ value: NONE, items: [item('b')] });
  });

  it('puts a value the engine did not name after the named ones, in the order it is met', () => {
    const groups = groupsOf([item('a', { status: ['frozen'] }), item('b', { status: ['open'] }), item('c', { status: ['melted'] })], STATUS, read);
    expect(groups.map((g) => g.value)).toEqual(['doing', 'open', 'parked', 'frozen', 'melted']);
  });

  it('puts an item with several values in each of their groups', () => {
    const groups = groupsOf([item('a', { tags: ['ui', 'bug'] }), item('b', { tags: ['bug'] })], TAGS, read);
    expect(groups.map((g) => [g.value, g.items.map((i) => i.id)])).toEqual([
      ['ui', ['a']],
      ['bug', ['a', 'b']],
    ]);
  });
});

describe('a key to the next group or cell', () => {
  it('goes one group over, and nowhere at the end or from a group that is not there', () => {
    const groups = groupsOf([], STATUS, read);
    expect(neighbourGroup(groups, 'doing', 1)).toBe('open');
    expect(neighbourGroup(groups, 'open', -1)).toBe('doing');
    expect(neighbourGroup(groups, 'doing', -1)).toBeNull();
    expect(neighbourGroup(groups, 'parked', 1)).toBeNull();
    expect(neighbourGroup(groups, 'nowhere', 1)).toBeNull();
  });

  it('goes a row up or down and a column left or right in a matrix, and nowhere off its edge', () => {
    const rows = groupsOf([], SEVERITY, read);
    const columns = groupsOf([], STATUS, read);
    const at = { row: 'critical', column: 'open' };
    expect(neighbourCell(rows, columns, at, 'down')).toEqual({ row: 'recommended', column: 'open' });
    expect(neighbourCell(rows, columns, at, 'up')).toBeNull();
    expect(neighbourCell(rows, columns, at, 'left')).toEqual({ row: 'critical', column: 'doing' });
    expect(neighbourCell(rows, columns, at, 'right')).toEqual({ row: 'critical', column: 'parked' });
  });
});

describe('the cells of a matrix', () => {
  it('has every row with every column, an empty cell too, and an item of a list in each of its cells', () => {
    const items = [item('a', { severity: ['critical'], tags: ['ui', 'bug'] }), item('b', { severity: ['recommended'], tags: ['bug'] })];
    const cells = cellsOf(items, SEVERITY, TAGS, read);
    expect(cells.map((c) => [c.row, c.column, c.items.map((i) => i.id)])).toEqual([
      ['critical', 'ui', ['a']],
      ['critical', 'bug', ['a']],
      ['recommended', 'ui', []],
      ['recommended', 'bug', ['b']],
    ]);
  });
});
