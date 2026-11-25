<script lang="ts">
  import { drawableImageUrl } from './image-url';
  import type { ImageSource, Inline } from './markdown';
  import { markHidden } from './text';
  import Inlines from './Inlines.svelte';

  // `images` is the app's way to the files of `assets/`: without it every image stays its description. `inLink`:
  // an image inside a link is not a button of its own (a button inside a link is no valid element).
  let { items, images, inLink = false }: { items: Inline[]; images?: ImageSource; inLink?: boolean } = $props();

  const fileName = (src: string) => src.slice(src.lastIndexOf('/') + 1);
</script>

{#each items as item, i (i)}
  {#if item.t === 'text'}
    {markHidden(item.text)}
  {:else if item.t === 'code'}
    <code translate="no">{markHidden(item.text)}</code>
  {:else if item.t === 'br'}
    <br />
  {:else if item.t === 'em'}
    <em><Inlines items={item.children} {images} {inLink} /></em>
  {:else if item.t === 'strong'}
    <strong><Inlines items={item.children} {images} {inLink} /></strong>
  {:else if item.t === 'del'}
    <del><Inlines items={item.children} {images} {inLink} /></del>
  {:else if item.t === 'link'}
    {#if item.href}
      <a href={item.href} rel="noopener noreferrer nofollow" target="_blank"
        ><Inlines items={item.children} {images} inLink={true} /></a
      >{#if item.host}<span class="host"> ({item.host})</span>{/if}
    {:else}
      <Inlines items={item.children} {images} {inLink} />
    {/if}
  {:else if item.t === 'image'}
    {@const src = item.src}
    {#if src !== undefined && images}
      {@const url = drawableImageUrl(images.url(src))}
      {@const alt = markHidden(item.alt) || fileName(src)}
      {#if url === null}
        <span class="missing">{images.missing}: {alt}</span>
      {:else if url === undefined}
        <span class="alt">{alt}</span>
      {:else if images.open && !inLink}
        {@const open = images.open}
        <button type="button" class="img-open" title={alt} onclick={() => open(src, url, alt)}
          ><img class="md-img" src={url} {alt} decoding="async" /></button
        >
      {:else}
        <img class="md-img" src={url} {alt} decoding="async" />
      {/if}
    {:else}
      <span class="alt">{markHidden(item.alt)}</span>
    {/if}
  {/if}
{/each}

<style>
  a {
    color: var(--accent);
    overflow-wrap: anywhere;
  }
  code {
    background: var(--surface-2);
    border-radius: 3px;
    padding: 0 0.25em;
  }
  .alt,
  .host,
  .missing {
    color: var(--muted);
  }
  .missing {
    font-style: italic;
  }
  .md-img {
    display: inline-block;
    max-width: 100%;
    max-height: 24rem;
    height: auto;
    vertical-align: middle;
  }
  .img-open {
    display: inline-block;
    padding: 0;
    border: 0;
    background: none;
    cursor: zoom-in;
    max-width: 100%;
  }
</style>
