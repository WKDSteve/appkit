<script lang="ts">
  // The documentation of the app in one window: a search box, the documents that match on the left, the chosen one
  // rendered as Markdown on the right. The app brings the documents (`docs`) and opens the window (`open`, `docId` is
  // the document to start at); the window knows no file. A click beside it or Escape closes it.
  import { closeOnBackdrop } from './backdrop';
  import { searchDocs, type DocEntry } from './docs';
  import { parseMarkdown } from './markdown';
  import Blocks from './Blocks.svelte';
  import type { FloatAction, FloatingParams } from './floating';

  interface Labels {
    title: string;
    /** The name of the search box. */
    label: string;
    placeholder: string;
    none: string;
    /** The name of the list of documents. */
    list: string;
    close: string;
  }

  const stayPut: FloatAction = () => ({ update() {}, destroy() {} });

  let {
    open,
    docs,
    docId = null,
    onclose,
    labels,
    float = stayPut,
    floatParams = { kind: 'docsdialog', moveLabel: '', moveHint: '' },
  }: {
    /** Makes the dialog movable and sizable: the action of the app (`floatingIn`), with its words. */
    float?: FloatAction;
    floatParams?: FloatingParams;
    open: boolean;
    docs: readonly DocEntry[];
    /** The document the window starts at; the first one when it is null or unknown. */
    docId?: string | null;
    onclose: () => void;
    labels: Labels;
  } = $props();

  let dialog: HTMLDialogElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let query = $state('');
  let chosen = $state<string | null>(null);

  const hits = $derived(searchDocs(docs, query));
  const current = $derived(hits.find((h) => h.doc.id === chosen)?.doc ?? hits[0]?.doc ?? null);
  const blocks = $derived(current ? parseMarkdown(current.text) : []);

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) {
      query = '';
      chosen = docId;
      dialog.showModal();
      // The search entry has no document to read yet: the box takes the focus.
      if (docId === null) input?.focus();
    }
    if (!open && dialog.open) dialog.close();
  });
</script>

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
  <!-- Nothing of the documents is read or drawn while the window is shut. -->
  {#if open}
    <header>
      <h2>{labels.title}</h2>
      <button type="button" class="close" aria-label={labels.close} onclick={() => dialog?.close()}>×</button>
    </header>
    <input
      bind:this={input}
      type="search"
      aria-label={labels.label}
      autocomplete="off"
      spellcheck="false"
      placeholder={labels.placeholder}
      bind:value={query}
    />
    <div class="body">
      <ul aria-label={labels.list}>
        {#each hits as hit (hit.doc.id)}
          <li>
            <button type="button" class:chosen={hit.doc.id === current?.id} aria-current={hit.doc.id === current?.id} onclick={() => (chosen = hit.doc.id)}>
              <span class="name">{hit.doc.title}</span>
              {#if hit.snippet}<span class="snippet">{hit.snippet}</span>{/if}
            </button>
          </li>
        {:else}
          <li class="none">{labels.none}</li>
        {/each}
      </ul>
      <article class="doc">
        {#if current}<Blocks {blocks} />{/if}
      </article>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    padding: 0.75rem 1rem 1rem;
    width: min(60rem, calc(100vw - 2rem));
    height: min(40rem, calc(100vh - 4rem));
    box-sizing: border-box;
  }
  dialog[open] {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  dialog::backdrop {
    background: var(--backdrop);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 1.1rem;
  }
  .close {
    background: none;
    border: 0;
    color: var(--muted);
    font-size: 1.4rem;
    line-height: 1;
    cursor: pointer;
    border-radius: var(--radius);
  }
  input {
    box-sizing: border-box;
    width: 100%;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.45rem 0.6rem;
    font: inherit;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(12rem, 18rem) minmax(0, 1fr);
    gap: 1rem;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow: auto;
  }
  li button {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    color: inherit;
    border: 0;
    border-radius: var(--radius);
    padding: 0.4rem 0.5rem;
    font: inherit;
    cursor: pointer;
  }
  li button:hover,
  li button.chosen {
    background: var(--surface-2);
  }
  li button.chosen {
    outline: 1px solid var(--accent);
  }
  .name {
    display: block;
    font-weight: 600;
  }
  .snippet {
    display: block;
    color: var(--muted);
    font-size: 0.78rem;
    overflow-wrap: anywhere;
  }
  .none {
    color: var(--muted);
    padding: 0.4rem 0.5rem;
  }
  .doc {
    overflow: auto;
    min-width: 0;
    padding-right: 0.5rem;
  }
  @media (max-width: 40rem) {
    .body {
      grid-template-columns: 1fr;
    }
  }
</style>
