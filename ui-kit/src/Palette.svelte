<script lang="ts">
  // The command palette (Ctrl+K): one box, the actions of the page and its entries in one list. The arrow keys
  // choose, Enter runs, Escape closes. The page works out the list (`items`), runs the choice (`onchoose`) and
  // remembers it; this component is the box, the list and the keys.
  import { closeOnBackdrop } from './backdrop';
  import { stepIndex, type PaletteItem } from './palette';
  import { markHidden } from './text';
  import type { FloatAction, FloatingParams } from './floating';

  interface Labels {
    /** The name of the dialog and of the list. */
    title: string;
    /** The name of the input. */
    label: string;
    placeholder: string;
    /** Shown when nothing matches. */
    none: string;
    /** The small word before an action, and before an entry (`Task`, `Rule`). */
    action: string;
    entry: string;
  }

  const stayPut: FloatAction = () => ({ update() {}, destroy() {} });

  let {
    open,
    items,
    onchoose,
    onclose,
    labels,
    show = markHidden,
    float = stayPut,
    floatParams = { kind: 'palette', moveLabel: '', moveHint: '' },
  }: {
    /** Makes the dialog movable and sizable: the action of the app (`floatingIn`), with its words. */
    float?: FloatAction;
    floatParams?: FloatingParams;
    open: boolean;
    /** The list for the words in the box; read inside the component, so the page's own state is tracked. */
    items: (query: string) => PaletteItem[];
    onchoose: (item: PaletteItem) => void | Promise<void>;
    onclose: () => void;
    labels: Labels;
    /** How a text from the data is made safe to show. */
    show?: (text: string) => string;
  } = $props();

  let dialog: HTMLDialogElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let query = $state('');
  let at = $state(0);

  const shown = $derived(open ? items(query) : []);
  // The choice stays inside the list when the words narrow it.
  $effect(() => {
    if (at >= shown.length) at = Math.max(0, shown.length - 1);
  });

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) {
      query = '';
      at = 0;
      dialog.showModal();
      input?.focus();
    }
    if (!open && dialog.open) dialog.close();
  });

  function choose(item: PaletteItem | undefined): void {
    if (item) void onchoose(item);
  }

  function onKey(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      at = stepIndex(at, event.key === 'ArrowDown' ? 1 : -1, shown.length);
      document.getElementById(`palette-option-${at}`)?.scrollIntoView({ block: 'nearest' });
    } else if (event.key === 'Enter') {
      event.preventDefault();
      choose(shown[at]);
    }
  }
</script>

<!--
  `close` comes a moment after the dialog is shut: a palette opened again at once (Ctrl+K right after Escape) must
  not be closed by the event of the one before, so only a dialog that is shut now counts.
-->
<dialog
  use:float={floatParams}
  bind:this={dialog}
  aria-label={labels.title}
  use:closeOnBackdrop
  oncancel={() => onclose()}
  onclose={() => {
    if (!dialog?.open) onclose();
  }}
>
  <input
    bind:this={input}
    type="text"
    role="combobox"
    aria-label={labels.label}
    aria-expanded="true"
    aria-controls="palette-list"
    aria-activedescendant={shown.length > 0 ? `palette-option-${at}` : undefined}
    aria-autocomplete="list"
    autocomplete="off"
    spellcheck="false"
    placeholder={labels.placeholder}
    bind:value={query}
    oninput={() => (at = 0)}
    onkeydown={onKey}
  />
  <ul id="palette-list" role="listbox" aria-label={labels.title}>
    {#each shown as item, i (item.key)}
      <li
        id={`palette-option-${i}`}
        role="option"
        aria-selected={i === at}
        class:chosen={i === at}
        onmousemove={() => (at = i)}
        onclick={() => choose(item)}
        onkeydown={null}
      >
        <span class="kind">{item.kind === 'entry' ? labels.entry : labels.action}</span>
        <span class="label">{show(item.label)}</span>
        {#if item.hint}<span class="hint">{show(item.hint)}</span>{/if}
      </li>
    {:else}
      <li class="none" role="presentation">{labels.none}</li>
    {/each}
  </ul>
</dialog>

<style>
  dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.5rem;
    width: min(38rem, calc(100vw - 2rem));
    margin-top: 12vh;
  }
  dialog::backdrop {
    background: var(--backdrop);
  }
  input {
    width: 100%;
    box-sizing: border-box;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.5rem 0.6rem;
    font: inherit;
  }
  ul {
    list-style: none;
    margin: 0.4rem 0 0;
    padding: 0;
    max-height: 50vh;
    overflow: auto;
  }
  li {
    display: grid;
    grid-template-columns: 4.5rem minmax(0, 1fr) auto;
    gap: 0.5rem;
    align-items: baseline;
    padding: 0.35rem 0.5rem;
    border-radius: var(--radius);
    cursor: pointer;
  }
  li.chosen {
    background: var(--surface-2);
    outline: 1px solid var(--accent);
  }
  .kind {
    color: var(--muted);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    color: var(--muted);
    font-size: 0.8rem;
  }
  .none {
    color: var(--muted);
    cursor: default;
    display: block;
  }
</style>
