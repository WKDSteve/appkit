<script lang="ts">
  import { closeOnBackdrop } from './backdrop';
  // The conflict center: every change the engine refused because the file had moved on. For each, what was wanted
  // beside what is there now, and the actions the app offers (try again on the version that is there, look at it,
  // let it go). Nothing is merged or written here. The person chooses how it is shown: every conflict in a list
  // (the default), or the first one with a count of the rest.
  import type { ConflictItem, ConflictMode } from './conflicts';
  import { markHidden } from './text';
  import type { FloatAction, FloatingParams } from './floating';

  interface Labels {
    title: string;
    intro: string;
    field: string;
    wanted: string;
    now: string;
    /** "{n} more": filled by the app. */
    more: (n: number) => string;
    discardAll: string;
    close: string;
    /** The label over the choice of the way it is shown, and its two words. */
    view: string;
    viewList: string;
    viewSingle: string;
  }

  const stayPut: FloatAction = () => ({ update() {}, destroy() {} });

  let {
    open,
    items,
    mode,
    onmode,
    onaction,
    ondiscardall,
    onclose,
    oncancel,
    labels,
    show = markHidden,
    float = stayPut,
    floatParams = { kind: 'conflictcenter', moveLabel: '', moveHint: '' },
  }: {
    /** Makes the dialog movable and sizable: the action of the app (`floatingIn`), with its words. */
    float?: FloatAction;
    floatParams?: FloatingParams;
    open: boolean;
    items: readonly ConflictItem[];
    mode: ConflictMode;
    onmode: (mode: ConflictMode) => void;
    onaction: (key: string, action: string) => void | Promise<void>;
    /** Without it there is no button for letting all go. */
    ondiscardall?: () => void;
    onclose: () => void;
    /** Escape. The app may make it a choice (rules-desktop drops the conflict, or does nothing); the default closes. */
    oncancel?: () => void;
    labels: Labels;
    show?: (text: string) => string;
  } = $props();

  let dialog: HTMLDialogElement | undefined = $state();

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  });

  const shown = $derived(mode === 'single' ? items.slice(0, 1) : items);
</script>

<dialog
  use:float={floatParams}
  bind:this={dialog}
  aria-labelledby="conflicts-title"
  use:closeOnBackdrop
  oncancel={(event) => {
    event.preventDefault();
    // Read when the key is pressed, so that a handler the app changed meanwhile is the one that runs.
    (oncancel ?? onclose)();
  }}
  onclose={() => {
    if (!dialog?.open) onclose();
  }}
>
  <h2 id="conflicts-title">{labels.title} ({items.length})</h2>
  <p class="intro">{labels.intro}</p>
  <div class="mode" role="group" aria-label={labels.view}>
    <button type="button" aria-pressed={mode === 'list'} onclick={() => onmode('list')}>{labels.viewList}</button>
    <button type="button" aria-pressed={mode === 'single'} onclick={() => onmode('single')}>{labels.viewSingle}</button>
  </div>
  {#if mode === 'single' && items.length > 1}<p class="more">{labels.more(items.length - 1)}</p>{/if}
  <ul>
    {#each shown as item (item.key)}
      <li>
        <p class="what">{show(item.title)}</p>
        {#if item.id}<p class="id"><code>{show(item.id)}</code></p>{/if}
        {#each item.notes as note, i (i)}<p class="note">{note}</p>{/each}
        {#if item.rows.length > 0}
          <table>
            <thead>
              <tr>
                <th scope="col"><span class="sr-only">{labels.field}</span></th>
                <th scope="col">{labels.wanted}</th>
                <th scope="col">{labels.now}</th>
              </tr>
            </thead>
            <tbody>
              {#each item.rows as row (row.key)}
                <tr>
                  <th scope="row">{show(row.label)}</th>
                  <td><pre>{show(row.wanted)}</pre></td>
                  <td class:same={row.wanted === row.now}><pre>{show(row.now)}</pre></td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
        <div class="actions">
          {#each item.actions as action (action.name)}
            <button
              type="button"
              class:primary={action.primary}
              disabled={action.disabled}
              onclick={() => void onaction(item.key, action.name)}>{action.label}</button
            >
          {/each}
        </div>
      </li>
    {/each}
  </ul>
  <div class="actions foot">
    {#if ondiscardall}<button type="button" onclick={() => ondiscardall()}>{labels.discardAll}</button>{/if}
    <button type="button" onclick={() => onclose()}>{labels.close}</button>
  </div>
</dialog>

<style>
  dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 1rem 1.25rem;
    width: min(40rem, calc(100vw - 2rem));
    max-height: 80vh;
    overflow: auto;
  }
  dialog::backdrop {
    background: var(--backdrop);
  }
  h2 {
    margin: 0 0 0.4rem;
    font-size: 1.1rem;
  }
  .intro,
  .more {
    color: var(--muted);
    font-size: 0.85rem;
    margin: 0 0 0.6rem;
  }
  .mode {
    display: flex;
    gap: 0.3rem;
    margin: 0 0 0.6rem;
  }
  .mode button[aria-pressed='true'] {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.7rem;
  }
  li {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.6rem 0.8rem;
  }
  .what {
    font-weight: 600;
    margin: 0;
  }
  .id {
    margin: 0.1rem 0 0.4rem;
    color: var(--muted);
  }
  .note {
    margin: 0.3rem 0;
    color: var(--muted);
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 0.85rem;
  }
  th,
  td {
    text-align: left;
    vertical-align: top;
    padding: 0.15rem 0.5rem 0.15rem 0;
  }
  thead th {
    color: var(--muted);
    font-weight: normal;
  }
  pre {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-family: inherit;
  }
  .same {
    color: var(--muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }
  .foot {
    margin-top: 0.9rem;
  }
  button {
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.35rem 0.8rem;
    cursor: pointer;
  }
  .primary {
    border-color: var(--accent);
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
