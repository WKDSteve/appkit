<script lang="ts">
  // The context menu of an app: one component for the whole page. A right click, the context key and Shift+F10 open it
  // on the targets around the pointer or the focus (`use:contextTarget`); what it offers is what the providers of those
  // targets say (`contextmenu.ts`). The browser's own menu stays where nobody offers anything, and in text fields.
  // Keys: Up, Down, Home and End walk the entries, Enter and Space choose, Escape closes and gives the focus back to the
  // element the menu was opened on. A click beside it, a scroll that moves its target and a resize close it.
  import { tick } from 'svelte';
  import { ariaShortcut, flatItems, toBlocks, TYPE_AHEAD_MS, typeAheadIndex, type MenuItem, type PlainItem } from './menu';
  import {
    buildContextMenu,
    isEditable,
    placeMenu,
    targetChain,
    type ContextRegistry,
    type ContextTarget,
  } from './contextmenu';
  import { tooltip } from './tooltip';

  let {
    registry,
    label,
    selection = () => [],
  }: {
    registry: ContextRegistry;
    /** The name of the menu for a screen reader, in the language of the page. */
    label: string;
    /** The targets that are selected right now; a target that is one of them hands the whole selection on. */
    selection?: () => readonly ContextTarget[];
  } = $props();

  let shown = $state<{ x: number; y: number; items: PlainItem[]; back: HTMLElement | null; placed: boolean } | null>(null);
  let menu: HTMLElement | undefined = $state();
  // A list longer than the window scrolls inside itself; a scroll box needs a way in for the keyboard (WCAG 2.1.1).
  let scrolls = $state(false);
  // Where the target of the open menu was when it opened (a scroll that leaves it there does not close the menu).
  let anchor: { el: HTMLElement; box: DOMRect } | null = null;
  const uid = $props.id();
  let typed = '';
  let typedTimer: ReturnType<typeof setTimeout> | undefined;

  const roleOf = (item: MenuItem): string =>
    item.kind === 'radio' ? 'menuitemradio' : item.kind === 'check' ? 'menuitemcheckbox' : 'menuitem';

  function buttons(): HTMLButtonElement[] {
    return menu ? [...menu.querySelectorAll<HTMLButtonElement>('button.cm-item:not(:disabled)')] : [];
  }

  /**
   * Show `items` at a point. The menu is drawn once out of sight at the corner of the window, where it has its natural
   * width (a box at the right edge would be squeezed), measured, and then put where it fits.
   */
  async function show(items: MenuItem[], at: { x: number; y: number }, back: HTMLElement | null): Promise<void> {
    // A submenu does not open here: its items stand in the list under its label.
    shown = { x: 0, y: 0, items: flatItems(items), back, placed: false };
    anchor = back && { el: back, box: back.getBoundingClientRect() };
    await tick();
    if (menu) {
      const place = placeMenu(at, menu.getBoundingClientRect(), { width: window.innerWidth, height: window.innerHeight });
      shown = shown && { ...shown, x: place.x, y: place.y, placed: true };
    }
    await tick();
    scrolls = !!menu && menu.scrollHeight > menu.clientHeight;
    buttons()[0]?.focus();
  }

  /**
   * Whether the page scrolled the menu's target away. A scroll event comes a frame after the scroll: one from before the
   * click (a smooth scroll, a box scrolled to the target) arrives when the menu is already open, and leaves the target
   * where it was; so does a scroll the browser makes to keep the view in place when the page above it changes.
   */
  function scrolledAway(): boolean {
    if (!anchor || !anchor.el.isConnected) return true;
    const now = anchor.el.getBoundingClientRect();
    return Math.abs(now.top - anchor.box.top) > 1 || Math.abs(now.left - anchor.box.left) > 1;
  }

  /**
   * The menu lives where its target lives: in an open dialog it is drawn inside that dialog. A modal dialog lies in the
   * top layer and makes the rest of the page inert, so a menu in the body would be behind it and could not be reached
   * (not even as a popover, which would be drawn on top but stay inert). The tooltip does the same (`tooltip.ts`).
   * The node is the only one of its block, so Svelte removes it wherever it is.
   */
  function inDialogOf(node: HTMLElement, back: HTMLElement | null): { destroy: () => void } {
    const dialog = back?.closest('dialog');
    if (!dialog?.open) return { destroy: () => {} };
    if (dialog !== node.parentElement) dialog.append(node);
    // The dialog closes under the open menu (its own button, the app): the menu goes with it.
    const gone = (): void => close(false);
    dialog.addEventListener('close', gone);
    return { destroy: () => dialog.removeEventListener('close', gone) };
  }

  function close(restore: boolean): void {
    const back = shown?.back ?? null;
    shown = null;
    if (restore && back && back.isConnected) back.focus();
  }

  function onContextMenu(event: MouseEvent): void {
    if (event.defaultPrevented) return;
    const el = event.target instanceof Element ? event.target : null;
    if (isEditable(el)) {
      close(false);
      return;
    }
    const chain = targetChain(el).chain;
    if (chain.length === 0) {
      close(false);
      return;
    }
    // Decided before the event is over: the browser's menu is replaced only by a menu that has something to say.
    const items = buildContextMenu(registry, chain, selection());
    if (toBlocks(items).length === 0) {
      close(false);
      return;
    }
    event.preventDefault();
    void show(items, { x: event.clientX, y: event.clientY }, targetChain(el).element);
  }

  function onKey(event: KeyboardEvent): void {
    if (shown) {
      onMenuKey(event);
      return;
    }
    const byKey = event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey);
    if (!byKey || event.isComposing) return;
    const active = document.activeElement;
    if (!active || isEditable(active)) return;
    const { chain, element } = targetChain(active);
    if (chain.length === 0) return;
    const items = buildContextMenu(registry, chain, selection());
    if (toBlocks(items).length === 0) return;
    event.preventDefault();
    const box = (element ?? (active as HTMLElement)).getBoundingClientRect();
    void show(items, { x: box.left, y: box.bottom }, active as HTMLElement);
  }

  function onMenuKey(event: KeyboardEvent): void {
    const list = buttons();
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === 'Escape') {
      event.preventDefault();
      close(true);
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const delta = event.key === 'ArrowDown' ? 1 : -1;
      // A menu wraps: past the last entry is the first.
      list[at < 0 ? (delta > 0 ? 0 : list.length - 1) : (at + delta + list.length) % list.length]?.focus();
    } else if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      list[event.key === 'Home' ? 0 : list.length - 1]?.focus();
    } else if (event.key === 'Tab') {
      close(false);
    } else if ([...event.key].length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey && event.key !== ' ') {
      // A typed letter goes to the next entry that starts with it (a word, to the one it names).
      clearTimeout(typedTimer);
      typed += event.key;
      typedTimer = setTimeout(() => (typed = ''), TYPE_AHEAD_MS);
      const to = typeAheadIndex(
        list.map((b) => b.textContent?.replace(/^[●✓\s]+/, '').trim() ?? ''),
        at,
        typed,
      );
      // A letter that names nothing is still not a shortcut of the page behind the menu.
      event.preventDefault();
      if (to >= 0) list[to]?.focus();
    }
  }

  function pick(item: PlainItem): void {
    if (item.kind === 'info' || item.kind === 'section' || item.kind === 'separator') return;
    if ((item.kind === 'action' || item.kind === 'radio') && item.disabled) return;
    close(true);
    item.onselect();
  }
</script>

<svelte:window
  oncontextmenu={onContextMenu}
  onkeydowncapture={onKey}
  onresize={() => close(false)}
  onblur={() => close(false)}
  onmousedown={(e) => {
    if (shown && !(e.target instanceof Node && menu?.contains(e.target))) close(false);
  }}
  onscrollcapture={(e) => {
    // A long menu scrolls inside itself; the page scrolling its target away closes it.
    if (shown && !(e.target instanceof Node && menu?.contains(e.target)) && scrolledAway()) close(false);
  }}
/>

{#snippet rows(items: PlainItem[])}
  {#each items as item, i (i)}
    {#if item.kind === 'info'}
      <p class="cm-note" role="none">{item.label}</p>
    {:else if item.kind !== 'separator' && item.kind !== 'section'}
      <button
        type="button"
        class="cm-item"
        role={roleOf(item)}
        aria-checked={item.kind === 'radio' || item.kind === 'check' ? item.checked : undefined}
        aria-keyshortcuts={item.shortcut ? ariaShortcut(item.shortcut) : undefined}
        disabled={(item.kind === 'action' || item.kind === 'radio') && item.disabled}
        tabindex="-1"
        use:tooltip={item.hint ?? ''}
        onclick={() => pick(item)}
      >
        <span class="cm-mark" aria-hidden="true"
          >{item.kind === 'radio' ? (item.checked ? '●' : '') : item.kind === 'check' ? (item.checked ? '✓' : '') : ''}</span
        >
        <span class="cm-text">{item.label}</span>
        {#if item.shortcut}<span class="cm-shortcut">{item.shortcut}</span>{/if}
      </button>
    {/if}
  {/each}
{/snippet}

{#if shown}
  <div
    class="cm-popup"
    role="menu"
    aria-label={label}
    bind:this={menu}
    use:inDialogOf={shown.back}
    tabindex={scrolls ? 0 : undefined}
    style:left="{shown.x}px"
    style:top="{shown.y}px"
    style:visibility={shown.placed ? undefined : 'hidden'}
  >
    {#each toBlocks(shown.items) as block, b (b)}
      {#if block.kind === 'divider'}
        <hr />
      {:else if block.heading !== null}
        <div role="group" aria-labelledby="{uid}-{b}">
          <p class="cm-section" id="{uid}-{b}" role="presentation">{block.heading}</p>
          {@render rows(block.items)}
        </div>
      {:else}
        {@render rows(block.items)}
      {/if}
    {/each}
  </div>
{/if}

<style>
  .cm-popup {
    position: fixed;
    z-index: 40;
    min-width: 13rem;
    max-width: calc(100vw - 1rem);
    max-height: calc(100vh - 1rem);
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    padding: 0.3rem;
  }
  .cm-item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    text-align: left;
    background: none;
    border: 0;
    border-radius: var(--radius);
    padding: 0.3rem 0.5rem;
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
  }
  /* As in the menu bar: the entry under the pointer or the key is filled with the accent. */
  .cm-item:hover:not(:disabled),
  .cm-item:focus-visible {
    background: var(--accent);
    color: var(--accent-text);
  }
  .cm-item:focus-visible {
    outline: 2px solid transparent;
    outline-offset: -2px;
  }
  .cm-item:hover:not(:disabled) .cm-mark,
  .cm-item:focus-visible .cm-mark,
  .cm-item:hover:not(:disabled) .cm-shortcut,
  .cm-item:focus-visible .cm-shortcut {
    color: inherit;
  }
  .cm-item:disabled {
    color: var(--muted);
    cursor: default;
  }
  .cm-text {
    flex: 1;
  }
  .cm-shortcut {
    margin-left: 1.5rem;
    color: var(--muted);
    font-size: 0.85rem;
  }
  .cm-mark {
    width: 1rem;
    text-align: center;
    color: var(--accent);
  }
  .cm-section {
    margin: 0;
    padding: 0.35rem 0.5rem 0.15rem;
    color: var(--muted);
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .cm-note {
    margin: 0;
    padding: 0.3rem 0.5rem;
    color: var(--muted);
    font-size: 0.85rem;
  }
  hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 0.3rem 0;
  }
</style>
