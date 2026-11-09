// Groups: the items of a view laid in groups by a field, and the moves between them. Nothing here knows a field or an
// item: which fields there are and what values each can have is the engine's (a document such as `tasks.groupings`),
// and the value an item has for a field is read by the app (`valuesOf`). What a drop onto a group writes is the app's
// too: this lays the groups, the cells of a matrix and the neighbours a key goes to.

/** What a grouping must say for the groups to be laid: the values the engine names, in its order. */
export interface GroupingLike {
  values: readonly string[];
}

/** The value that stands for "this item has none" (a group of its own: a target too). */
export const NONE = '';

/** One group: its value (`NONE` for the items that have none) and the items that are in it. */
export interface Group<T> {
  value: string;
  items: T[];
}

/** The values an item has for a grouping: one, several (a list such as tags), or none. */
export type ValuesOf<T, G extends GroupingLike = GroupingLike> = (item: T, grouping: G) => readonly string[];

/**
 * The items in groups: one group for every value the engine names, in its order (an empty group stays: it is a place to
 * drop), a value the engine did not name after them in the order it is met, and the items that have none last. An item
 * with several values stands in each of their groups; the group of none shows only when some item has none.
 */
export function groupsOf<T, G extends GroupingLike>(items: readonly T[], grouping: G, valuesOf: ValuesOf<T, G>): Group<T>[] {
  const groups = new Map<string, T[]>();
  for (const value of grouping.values) groups.set(value, []);
  const unnamed: string[] = [];
  const none: T[] = [];
  for (const item of items) {
    const values = valuesOf(item, grouping);
    if (values.length === 0) {
      none.push(item);
      continue;
    }
    for (const value of values) {
      let list = groups.get(value);
      if (!list) {
        list = [];
        groups.set(value, list);
        unnamed.push(value);
      }
      list.push(item);
    }
  }
  const out: Group<T>[] = [...grouping.values, ...unnamed].map((value) => ({ value, items: groups.get(value) ?? [] }));
  if (none.length > 0) out.push({ value: NONE, items: none });
  return out;
}

/** The count a group shows: how many items are in it. */
export function countOf(group: Group<unknown>): number {
  return group.items.length;
}

/** The group before (`-1`) or after (`1`) `value`, or null at the end: where a key moves a card to. */
export function neighbourGroup(groups: readonly Group<unknown>[], value: string, dir: 1 | -1): string | null {
  const at = groups.findIndex((g) => g.value === value);
  if (at < 0) return null;
  return groups[at + dir]?.value ?? null;
}

/** A cell of a matrix: the value of its row and of its column. */
export interface Cell {
  row: string;
  column: string;
}

/** The cell next to `at` in a matrix: a row up or down (`rows`), a column to the left or right, or null at the edge. */
export function neighbourCell(
  rows: readonly Group<unknown>[],
  columns: readonly Group<unknown>[],
  at: Cell,
  move: 'up' | 'down' | 'left' | 'right',
): Cell | null {
  if (move === 'up' || move === 'down') {
    const row = neighbourGroup(rows, at.row, move === 'down' ? 1 : -1);
    return row === null ? null : { row, column: at.column };
  }
  const column = neighbourGroup(columns, at.column, move === 'right' ? 1 : -1);
  return column === null ? null : { row: at.row, column };
}

/**
 * The two fields of a matrix: the cells of rows by columns, each with the items in both. Rows and columns are the groups
 * of the two groupings over all the items (so every row has the same columns, an empty cell included); an item of a
 * list field stands in every cell it belongs to.
 */
export function cellsOf<T extends { id: string }, G extends GroupingLike>(
  items: readonly T[],
  rows: G,
  columns: G,
  valuesOf: ValuesOf<T, G>,
): { row: string; column: string; items: T[] }[] {
  const columnGroups = groupsOf(items, columns, valuesOf).map((g) => ({ value: g.value, ids: new Set(g.items.map((i) => i.id)) }));
  const out: { row: string; column: string; items: T[] }[] = [];
  for (const row of groupsOf(items, rows, valuesOf)) {
    for (const column of columnGroups) {
      out.push({ row: row.value, column: column.value, items: row.items.filter((i) => column.ids.has(i.id)) });
    }
  }
  return out;
}
