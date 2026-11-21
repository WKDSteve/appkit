<script lang="ts" module>
  export type { Menu, MenuItem } from './menu';
</script>

<script lang="ts">
  // The menu bar of an application: a row of menus, each a button that opens a list. The list falls into sections
  // (a heading and a line), an item may show its shortcut on the right. Keys: Enter, Space or Down open a menu;
  // Up and Down walk the items; a letter goes to the next item that starts with it; Left and Right go to the next
  // menu; Escape closes; Alt+letter opens the menu whose `mnemonic` that is. The shortcut is only shown: the app
  // binds the key itself. An item with `onopennew` opens apart like a link: middle click, Ctrl+click, Ctrl+Enter.
  // An item with `href` is a link (`<a>`): what the page does not take itself (the context menu, Shift+click, a middle
  // click or Ctrl+click without `onopennew`) the browser does with the address, a plain click stays in the page.

  import { onMount, tick, untrack } from 'svelte';
  import { tooltip } from './tooltip';
  import {
    anchorName,
    ariaShortcut,
    canAnchorPopover,
    letterOfCode,
    menuForMnemonic,
    popupShift,
    splitMnemonic,
    toBlocks,
    TYPE_AHEAD_MS,
    typeAheadIndex,
    type Menu,
    type MenuItem,
  } from './menu';

  let {
    menus,
    label,
    initialOpen = null,
  }: {
    menus: readonly Menu[];
    label: string;
    /** The id of a menu that is open from the start (a page that renders the list at once, a test). */
    initialOpen?: string | null;
  } = $props();

  // Stable between the server and the client (Math.random would differ on hydration).
  const uid = $props.id();
  // Where the platform can do it, the open popup is a popover in the top layer, anchored to its title by CSS (it flips
  // to stay in the window, and no box of the page clips or covers it). Elsewhere it stays where the page puts it.
  const anchored =
    typeof window !== 'undefined' &&
    canAnchorPopover({ showPopover: HTMLElement.prototype.showPopover, supports: (c) => CSS.supports(c) });
  let openId = $state<string | null>(untrack(() => initialOpen));
  let typed = '';
  let typedTimer: ReturnType<typeof setTimeout> | undefined;
  // One tab stop for the whole bar (the ARIA menubar pattern): the trigger that was used last takes it, the others
  // are reached with Left and Right, Home and End.
  let stop = $state<string | null>(null);
  const stopId = $derived(stop !== null && menus.some((m) => m.id === stop) ? stop : (menus[0]?.id ?? null));
  let root: HTMLElement | undefined = $state();

  const roleOf = (item: MenuItem): string =>
    item.kind === 'radio' ? 'menuitemradio' : item.kind === 'check' ? 'menuitemcheckbox' : 'menuitem';

  function triggers(): HTMLButtonElement[] {
    return root ? [...root.querySelectorAll<HTMLButtonElement>('button.mb-trigger')] : [];
  }

  function itemButtons(id: string): HTMLElement[] {
    return root ? [...root.querySelectorAll<HTMLElement>(`[data-menu="${id}"] .mb-item:not(:disabled)`)] : [];
  }

  /** The address of an item that is a link, or null: it has none, or it cannot be chosen now (then it is a button). */
  function linkOf(item: MenuItem): string | null {
    if ((item.kind !== 'action' && item.kind !== 'radio') || item.disabled || !item.href) return null;
    return item.href;
  }

  function onItemClick(event: MouseEvent, item: MenuItem): void {
    if (apartKey(event) && pickApart(item)) {
      event.preventDefault();
      return;
    }
    // A link opened with a modifier is the browser's: a new tab or window at its address.
    if (linkOf(item) !== null && (apartKey(event) || event.shiftKey)) return;
    // Alt+click would save the link; a plain click goes to the place in this page, without loading it again.
    event.preventDefault();
    pick(item);
  }

  /** Tells the menu `id` that it opens (`Menu.onopen`). A hook that throws does not keep the menu shut. */
  function tell(id: string): void {
    try {
      menus.find((m) => m.id === id)?.onopen?.();
    } catch {
      // the menu opens with what it has
    }
  }

  // A menu that is open from the start (`initialOpen`) opened too: told once, when the bar is in the page (not on the
  // server, where nothing opens).
  onMount(() => {
    if (openId !== null) tell(openId);
  });

  async function open(id: string, focusItem: 'first' | 'last' | null): Promise<void> {
    // Told once per opening (not again when the keys move inside the open menu): the app reads again what the
    // menu shows and what can have changed since the page loaded.
    if (openId !== id) tell(id);
    openId = id;
    if (focusItem === null) return;
    await tick();
    const items = itemButtons(id);
    (focusItem === 'first' ? items[0] : items[items.length - 1])?.focus();
  }

  /**
   * Keeps an open popup inside the window: a menu far right in a narrow window would reach past the edge and give
   * the page a sideways scroll (then a scroll could put another title under the resting pointer). Measured once it
   * is drawn and again when the window changes size.
   */
  function fit(popup: HTMLElement): { destroy: () => void } {
    if (anchored) {
      // Anchored by CSS (see `anchored`): shown in the top layer, kept in the window by its fallbacks.
      popup.showPopover();
      return {
        destroy: () => {
          if (popup.matches(':popover-open')) popup.hidePopover();
        },
      };
    }
    const place = (): void => {
      popup.style.translate = '';
      const box = popup.getBoundingClientRect();
      const shift = popupShift(box.left, box.right, document.documentElement.clientWidth || window.innerWidth);
      if (shift !== 0) popup.style.translate = `${shift}px 0`;
    };
    place();
    window.addEventListener('resize', place);
    return { destroy: () => window.removeEventListener('resize', place) };
  }

  function close(focusTrigger: boolean): void {
    const was = openId;
    openId = null;
    if (focusTrigger && was) triggers().find((t) => t.dataset.for === was)?.focus();
  }

  function pick(item: MenuItem): void {
    if (item.kind === 'separator' || item.kind === 'info' || item.kind === 'section') return;
    if ((item.kind === 'action' || item.kind === 'radio') && item.disabled) return;
    close(true);
    item.onselect();
  }

  /** What an item opens apart (a new tab or window), or null: it has none, or it cannot be chosen now. */
  function apart(item: MenuItem): (() => void) | null {
    if ((item.kind !== 'action' && item.kind !== 'radio') || item.disabled || !item.onopennew) return null;
    return item.onopennew;
  }

  /** Open the item apart, as a link does with a middle click or a modifier; false when it has no such way. */
  function pickApart(item: MenuItem): boolean {
    const open = apart(item);
    if (!open) return false;
    close(true);
    open();
    return true;
  }

  // Ctrl on Windows and Linux, Cmd on a Mac: the modifier of "open in a new tab".
  const apartKey = (event: MouseEvent | KeyboardEvent): boolean => event.ctrlKey || event.metaKey;

  function onTriggerKey(event: KeyboardEvent, id: string): void {
    const list = triggers();
    const at = list.findIndex((t) => t.dataset.for === id);
    switch (event.key) {
      case 'ArrowDown':
      case 'Enter':
      case ' ':
        event.preventDefault();
        void open(id, 'first');
        break;
      case 'ArrowUp':
        event.preventDefault();
        void open(id, 'last');
        break;
      case 'ArrowRight':
      case 'ArrowLeft': {
        event.preventDefault();
        const step = event.key === 'ArrowRight' ? 1 : -1;
        const next = list[(at + step + list.length) % list.length];
        next?.focus();
        if (openId !== null && next?.dataset.for) void open(next.dataset.for, null);
        break;
      }
      case 'Home':
      case 'End': {
        event.preventDefault();
        const edge = list[event.key === 'Home' ? 0 : list.length - 1];
        edge?.focus();
        if (openId !== null && edge?.dataset.for) void open(edge.dataset.for, null);
        break;
      }
      case 'Escape':
        if (openId !== null) {
          event.preventDefault();
          event.stopPropagation();
          close(true);
        }
        break;
    }
  }

  function onItemKey(event: KeyboardEvent, id: string, item: MenuItem): void {
    if (event.key === 'Enter' && apartKey(event) && pickApart(item)) {
      event.preventDefault();
      return;
    }
    const items = itemButtons(id);
    const at = items.indexOf(event.currentTarget as HTMLElement);
    const focus = (i: number) => items[(i + items.length) % items.length]?.focus();
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        focus(at + 1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        focus(at - 1);
        break;
      case 'Home':
        event.preventDefault();
        focus(0);
        break;
      case 'End':
        event.preventDefault();
        focus(items.length - 1);
        break;
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        close(true);
        break;
      case 'ArrowRight':
      case 'ArrowLeft': {
        event.preventDefault();
        const list = triggers();
        const here = list.findIndex((t) => t.dataset.for === id);
        const step = event.key === 'ArrowRight' ? 1 : -1;
        const next = list[(here + step + list.length) % list.length];
        if (next?.dataset.for) void open(next.dataset.for, 'first');
        break;
      }
      case 'Tab':
        close(false);
        break;
      case ' ':
        // A button takes Space by itself; a link does not.
        if (linkOf(item) !== null) {
          event.preventDefault();
          pick(item);
        }
        break;
      default:
        if (event.key.length === 1 && event.key !== ' ' && !event.ctrlKey && !event.metaKey && !event.altKey) {
          event.preventDefault();
          clearTimeout(typedTimer);
          typed += event.key;
          typedTimer = setTimeout(() => (typed = ''), TYPE_AHEAD_MS);
          const labels = items.map((b) => b.querySelector('.mb-text')?.textContent?.trim() ?? '');
          const to = typeAheadIndex(labels, at, typed);
          if (to >= 0) items[to]?.focus();
        }
    }
  }

  function modalOpen(): boolean {
    try {
      return document.querySelector('dialog:modal') !== null;
    } catch {
      return false; // a browser that does not know :modal
    }
  }

  function onWindowKey(event: KeyboardEvent): void {
    if (event.defaultPrevented || !event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    // A modal dialog makes the page behind it inert: its keys are not the bar's.
    if (modalOpen()) return;
    const menu =
      menuForMnemonic(menus, event.key) ?? menuForMnemonic(menus, letterOfCode(event.code) ?? '');
    if (!menu) return;
    event.preventDefault();
    void open(menu.id, 'first');
  }

  function onWindowClick(event: MouseEvent): void {
    if (openId !== null && root && event.target instanceof Node && !root.contains(event.target)) {
      close(false);
    }
  }
</script>

{#snippet triggerText(menu: Menu)}
  {@const parts = splitMnemonic(menu.label, menu.mnemonic)}
  {parts.before}{#if parts.mark}<u>{parts.mark}</u>{/if}{parts.after}
{/snippet}

{#snippet rows(items: MenuItem[], id: string)}
  {#each items as item, i (i)}
    {#if item.kind === 'info'}
      <p class="mb-note" role="none">{item.label}</p>
    {:else if item.kind !== 'separator' && item.kind !== 'section'}
      {@const href = linkOf(item)}
      <svelte:element
        this={href !== null ? 'a' : 'button'}
        type={href !== null ? undefined : 'button'}
        href={href ?? undefined}
        class="mb-item"
        role={roleOf(item)}
        aria-checked={item.kind === 'radio' || item.kind === 'check' ? item.checked : undefined}
        aria-keyshortcuts={item.shortcut ? ariaShortcut(item.shortcut) : undefined}
        disabled={href !== null ? undefined : (item.kind === 'action' || item.kind === 'radio') && item.disabled}
        tabindex="-1"
        use:tooltip={item.hint ?? ''}
        onclick={(e: MouseEvent) => onItemClick(e, item)}
        onmousedown={(e: MouseEvent) => {
          // The middle button would start the browser's autoscroll on the page.
          if (e.button === 1 && apart(item)) e.preventDefault();
        }}
        onauxclick={(e: MouseEvent) => {
          if (e.button === 1 && pickApart(item)) e.preventDefault();
        }}
        onkeydown={(e: KeyboardEvent) => onItemKey(e, id, item)}
      >
        <span class="mb-mark" aria-hidden="true"
          >{item.kind === 'radio' ? (item.checked ? '●' : '') : item.kind === 'check' ? (item.checked ? '✓' : '') : ''}</span
        >
        <span class="mb-text">{item.label}</span>
        {#if item.shortcut}<span class="mb-shortcut">{item.shortcut}</span>{/if}
      </svelte:element>
    {/if}
  {/each}
{/snippet}

<svelte:window onclick={onWindowClick} onkeydown={onWindowKey} />

<!-- Focus that leaves the bar (Tab) closes the menu: it must not stay open behind. -->
<div
  class="mb-menubar"
  role="menubar"
  tabindex="-1"
  aria-label={label}
  bind:this={root}
  onfocusout={(e) => {
    const to = e.relatedTarget;
    if (openId !== null && root && !(to instanceof Node && root.contains(to))) close(false);
  }}
>
  {#each menus as menu (menu.id)}
    <div class="mb-menu">
      <button
        type="button"
        class="mb-trigger"
        role="menuitem"
        data-for={menu.id}
        style:anchor-name={anchored ? anchorName(`mb-${uid}`, menu.id) : undefined}
        use:tooltip={openId === menu.id ? '' : (menu.hint ?? '')}
        tabindex={stopId === menu.id ? 0 : -1}
        onfocus={() => (stop = menu.id)}
        aria-haspopup="menu"
        aria-keyshortcuts={menu.mnemonic ? `Alt+${menu.mnemonic.toLocaleUpperCase()}` : undefined}
        aria-expanded={openId === menu.id}
        onclick={() => (openId === menu.id ? close(false) : void open(menu.id, null))}
        onkeydown={(e) => onTriggerKey(e, menu.id)}
        onpointermove={(e) => {
          // A move of the pointer, not its enter: a scroll or a shift of the layout under a resting pointer brings
          // another title under it without a move, and must not switch the menu one is choosing in.
          if (e.pointerType !== 'touch' && openId !== null && openId !== menu.id) void open(menu.id, null);
        }}>{@render triggerText(menu)}</button
      >
      {#if openId === menu.id}
        <div
          class="mb-popup"
          role="menu"
          aria-label={menu.label}
          data-menu={menu.id}
          popover={anchored ? 'manual' : undefined}
          style:position-anchor={anchored ? anchorName(`mb-${uid}`, menu.id) : undefined}
          use:fit
        >
          {#each toBlocks(menu.items) as block, b (b)}
            {#if block.kind === 'divider'}
              <hr />
            {:else if block.heading !== null}
              <div role="group" aria-labelledby="{uid}-{menu.id}-{b}">
                <p class="mb-section" id="{uid}-{menu.id}-{b}" role="presentation">{block.heading}</p>
                {@render rows(block.items, menu.id)}
              </div>
            {:else}
              {@render rows(block.items, menu.id)}
            {/if}
          {/each}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .mb-menubar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.1rem;
  }
  .mb-trigger {
    white-space: nowrap;
  }
  .mb-trigger u {
    text-decoration: underline;
    text-underline-offset: 0.15em;
  }
  .mb-menu {
    position: relative;
  }
  .mb-trigger {
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius);
    padding: 0.25rem 0.6rem;
    cursor: pointer;
    color: var(--text);
  }
  .mb-trigger:hover,
  .mb-trigger[aria-expanded='true'] {
    background: var(--surface-2);
    border-color: var(--line);
  }
  .mb-popup {
    position: absolute;
    z-index: 20;
    top: calc(100% + 0.2rem);
    left: 0;
    min-width: 13rem;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    padding: 0.3rem;
    max-width: calc(100vw - 1rem);
    /* Portrait or a short window: a long list scrolls inside the popup instead of leaving the screen. */
    max-height: calc(100vh - 4rem);
    overflow-y: auto;
  }
  /* The last menu sits near the right edge in a narrow window: its popup opens to the left. */
  .mb-menu:last-child .mb-popup {
    left: auto;
    right: 0;
  }
  /* Anchored to its title (see `anchored`): a popover in the top layer, below the title, flipped to the other side
     where the window ends. The popover's own placement of the browser (centred, with a border) is taken back. */
  .mb-menu .mb-popup[popover],
  .mb-menu:last-child .mb-popup[popover] {
    position: fixed;
    inset: auto;
    margin: 0.2rem 0 0;
    color: inherit;
    position-area: block-end span-inline-end;
    position-try-fallbacks: flip-inline, flip-block, flip-block flip-inline;
  }
  .mb-item {
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
    /* A link item looks like the others. */
    font: inherit;
    text-decoration: none;
    box-sizing: border-box;
  }
  /* As in a native menu, the item under the pointer or the key is filled with the accent. */
  .mb-item:hover:not(:disabled),
  .mb-item:focus-visible {
    background: var(--accent);
    color: var(--accent-text);
  }
  /* The fill is the sign of the focus here (contrast of --accent-text on --accent): the ring of base.css on top of
     it would be a second frame around the same thing. The outline stays as a transparent one: in forced colours
     (Windows High Contrast) the fill is replaced, and a transparent outline is drawn there in a system colour. */
  .mb-item:focus-visible {
    outline: 2px solid transparent;
    outline-offset: -2px;
  }
  .mb-item:hover:not(:disabled) .mb-mark,
  .mb-item:focus-visible .mb-mark {
    color: inherit;
  }
  .mb-text {
    flex: 1;
  }
  .mb-shortcut {
    margin-left: 1.5rem;
    color: var(--muted);
    font-size: 0.85rem;
  }
  .mb-item:hover:not(:disabled) .mb-shortcut,
  .mb-item:focus-visible .mb-shortcut {
    color: inherit;
  }
  .mb-section {
    margin: 0;
    padding: 0.35rem 0.5rem 0.15rem;
    color: var(--muted);
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .mb-item:disabled {
    color: var(--muted);
    cursor: default;
  }
  .mb-mark {
    width: 1rem;
    text-align: center;
    color: var(--accent);
  }
  .mb-note {
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
