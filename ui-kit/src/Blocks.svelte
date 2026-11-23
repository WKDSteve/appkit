<script lang="ts">
  import type { Block, ImageSource } from './markdown';
  import { markHidden } from './text';
  import Blocks from './Blocks.svelte';
  import Inlines from './Inlines.svelte';

  // `headingBase` is the level of the heading the text sits under: its own headings start one level below it, so the
  // page keeps one order of headings (a text under an h2 starts at h3).
  // `images`: the app's way to the files below `assets/` of a text read with `parseMarkdown(…, { assets: true })`;
  // without it every image stays its description.
  let {
    blocks,
    headingBase = 1,
    images,
  }: { blocks: Block[]; headingBase?: number; images?: ImageSource } = $props();

  const heading = (level: number) => Math.min(6, level + headingBase);
</script>

{#each blocks as block, i (i)}
  {#if block.t === 'p'}
    <p><Inlines items={block.children} {images} /></p>
  {:else if block.t === 'h'}
    {@const level = heading(block.level)}
    <svelte:element this={`h${level}`} class="md-h"
      ><Inlines items={block.children} {images} /></svelte:element
    >
  {:else if block.t === 'list'}
    <svelte:element this={block.ordered ? 'ol' : 'ul'} start={block.ordered ? block.start : undefined}>
      {#each block.items as item, j (j)}
        <li><Blocks blocks={item} {headingBase} {images} /></li>
      {/each}
    </svelte:element>
  {:else if block.t === 'quote'}
    <blockquote><Blocks blocks={block.children} {headingBase} {images} /></blockquote>
  {:else if block.t === 'pre'}
    <!-- A scrolling block is reached with the keyboard. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <pre tabindex="0" translate="no"><code>{markHidden(block.text)}</code></pre>
  {:else if block.t === 'hr'}
    <hr />
  {:else if block.t === 'table'}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="scroll" tabindex="0">
      <table>
        {#if block.head.length > 0}
          <thead>
            <tr>
              {#each block.head as cell, j (j)}
                <th><Inlines items={cell} {images} /></th>
              {/each}
            </tr>
          </thead>
        {/if}
        <tbody>
          {#each block.rows as row, j (j)}
            <tr>
              {#each row as cell, k (k)}
                <td><Inlines items={cell} {images} /></td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{/each}

<style>
  p {
    margin: 0 0 0.6em;
    overflow-wrap: anywhere;
  }
  .md-h {
    font-size: 0.95rem;
    margin: 0.8em 0 0.4em;
  }
  ul,
  ol {
    margin: 0 0 0.6em;
    padding-left: 1.4em;
  }
  blockquote {
    margin: 0 0 0.6em;
    padding-left: 0.8em;
    border-left: 3px solid var(--line);
    color: var(--muted);
  }
  pre {
    background: var(--surface-2);
    border-radius: var(--radius);
    padding: 0.6rem 0.8rem;
    overflow: auto;
    margin: 0 0 0.6em;
  }
  .scroll {
    overflow: auto;
    margin: 0 0 0.6em;
  }
  table {
    border-collapse: collapse;
  }
  th,
  td {
    border: 1px solid var(--line);
    padding: 0.2rem 0.5rem;
    text-align: left;
  }
</style>
