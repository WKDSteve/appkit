<script lang="ts">
  // The items of a view in a matrix: a row for every value of one field, a column for every value of another, and in
  // each cell how many items have both. A count is a button that opens the cell (the caller shows its items); a cell
  // takes a card dropped on it (the caller turns that into the operation). Nothing here names a field or a value.
  import type { Cell, Group } from './groups';

  let {
    rows,
    columns,
    count,
    rowLabel,
    columnLabel,
    label,
    corner,
    cellLabel,
    opened = null,
    onopen,
    dragging = false,
    ondropcell,
    fingerOver = null,
  }: {
    rows: readonly Group<unknown>[];
    columns: readonly Group<unknown>[];
    /** How many items the cell holds. */
    count: (cell: Cell) => number;
    rowLabel: (value: string) => string;
    columnLabel: (value: string) => string;
    /** The name of the matrix for assistive software. */
    label: string;
    /** The words of the corner cell (which field runs down, which across). */
    corner: string;
    /** The words of a count button: the cell and its number, and that it opens. */
    cellLabel: (cell: Cell, n: number) => string;
    opened?: Cell | null;
    onopen: (cell: Cell) => void;
    /** A card of the view is being dragged: the cells take it. */
    dragging?: boolean;
    /** A card was dropped on `cell`; `copy` is the Ctrl key. */
    ondropcell?: (cell: Cell, copy: boolean) => void;
    /** The cell a finger that drags a card is over (the key of `data-cell`), lit as a drop with the mouse is. */
    fingerOver?: string | null;
  } = $props();

  let over = $state<string | null>(null);
  const keyOf = (cell: Cell) => JSON.stringify([cell.row, cell.column]);

  function enter(event: DragEvent, cell: Cell): void {
    if (!dragging || !ondropcell) return;
    event.preventDefault();
    over = keyOf(cell);
    if (event.dataTransfer) event.dataTransfer.dropEffect = event.ctrlKey ? 'copy' : 'move';
  }

  function drop(event: DragEvent, cell: Cell): void {
    event.preventDefault();
    over = null;
    if (dragging && ondropcell) ondropcell(cell, event.ctrlKey);
  }
</script>

<!-- A wide matrix scrolls in its own box, which the keyboard can reach. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="scroll" tabindex="0" role="region" aria-label={label}>
  <table>
    <thead>
      <tr>
        <th scope="col"><span class="corner">{corner}</span></th>
        {#each columns as column (column.value)}
          <th scope="col">{columnLabel(column.value)}</th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row (row.value)}
        <tr>
          <th scope="row">{rowLabel(row.value)}</th>
          {#each columns as column (column.value)}
            {@const cell = { row: row.value, column: column.value }}
            {@const n = count(cell)}
            <td
              class="num"
              class:target={over === keyOf(cell) || fingerOver === keyOf(cell)}
              class:opened={opened !== null && keyOf(opened) === keyOf(cell)}
              data-cell={keyOf(cell)}
              ondragenter={(e) => enter(e, cell)}
              ondragover={(e) => enter(e, cell)}
              ondragleave={() => (over = null)}
              ondrop={(e) => drop(e, cell)}
            >
              {#if n > 0}
                <button
                  type="button"
                  class="link"
                  aria-label={cellLabel(cell, n)}
                  aria-pressed={opened !== null && keyOf(opened) === keyOf(cell)}
                  onclick={() => onopen(cell)}>{n}</button
                >
              {:else}
                <span class="muted">0</span>
              {/if}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .scroll {
    overflow-x: auto;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.4rem 0.5rem;
  }
  table {
    border-collapse: collapse;
  }
  th,
  td {
    padding: 0.3rem 0.7rem;
    text-align: left;
    font-weight: normal;
  }
  thead th,
  .corner {
    color: var(--muted);
    font-size: 0.8rem;
  }
  tbody th {
    color: var(--muted);
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    border-radius: var(--radius);
  }
  .num.opened {
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .num.target {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    background: var(--accent-soft);
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
  }
  .muted {
    color: var(--muted);
  }
</style>
