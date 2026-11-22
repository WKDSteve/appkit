<script lang="ts" module>
  export type { Menu, MenuItem, SubmenuItem } from './menu';
</script>

<script lang="ts">
  // The menu bar of an application: a row of menus, each a button that opens a list. The list falls into sections
  // (a heading and a line), an item may show its shortcut on the right. Keys: Enter, Space or Down open a menu;
  // Up and Down walk the items; a letter goes to the next item that starts with it; Left and Right go to the next
  // menu; Escape closes; Alt+letter opens the menu whose `mnemonic` that is. The shortcut is only shown: the app
  // binds the key itself. An item with `onopennew` opens apart like a link: middle click, Ctrl+click, Ctrl+Enter.
  // An item with `href` is a link (`<a>`): what the page does not take itself (the context menu, Shift+click, a middle
  // click or Ctrl+click without `onopennew`) the browser does with the address, a plain click stays in the page.
  // A `submenu` item opens a list of its own (the ARIA menu pattern, one level): Right, Enter or Space open it and go
  // to its first item, Left and Escape close it and go back to its item; the pointer opens it after a short rest and
  // keeps it open on its way there. Beside the menu where there is room; folded open inside it in a narrow window.

  import { onMount, tick, untrack } from 'svelte';
  import { fill } from './i18n';
  import { tooltip } from './tooltip';
  import {
    anchorName,
    ariaShortcut,
    canAnchorPopover,
    flyoutPlace,
    inCorridor,
    letterOfCode,
    menuForMnemonic,
    popupShift,
    splitMnemonic,
    subItems,
    SUBMENU_GRACE_MS,
    SUBMENU_NARROW,
    SUBMENU_OPEN_MS,
    submenuCurrent,
    submenuUsable,
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
    labels = {},
  }: {
    menus: readonly Menu[];
    label: string;
    /** The id of a menu that is open from the start (a page that renders the list at once, a test). */
    initialOpen?: string | null;
    /**
     * Words of the bar. `submenuCurrent`: the name of a submenu item whose list has a checked radio item, with the
     * holes `{label}` and `{current}` (default `{label}, {current}`; German for instance `{label}, aktuell: {current}`).
     */
    labels?: { submenuCurrent?: string };
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

  // The open submenu, by the place of its item (`menu/block/index`), or null. One at a time, in the open menu only.
  let subOpen = $state<string | null>(null);
  const viewWidth = (): number =>
    typeof window === 'undefined' ? 1024 : document.documentElement.clientWidth || window.innerWidth;
  let vw = $state(viewWidth());
  // A narrow window folds a submenu open inside its menu: beside it, it would reach past the edge.
  const narrow = $derived(vw < SUBMENU_NARROW);
  // A submenu beside its menu is a popover in the top layer where the browser has one (no scrolling menu clips it);
  // elsewhere it is fixed to the window all the same, placed by `flyout`.
  const canPopover = typeof HTMLElement !== 'undefined' && typeof HTMLElement.prototype.showPopover === 'function';
  let openTimer: ReturnType<typeof setTimeout> | undefined;
  let openTimerKey: string | null = null;
  let graceTimer: ReturnType<typeof setTimeout> | undefined;
  // Where the pointer last was on the item of the open submenu: the tip of its corridor (`inCorridor`); and where it
  // was a move ago (a turn away from the submenu closes it at once).
  let origin = { x: 0, y: 0 };
  let last = { x: 0, y: 0 };
  const currentText = $derived(labels.submenuCurrent ?? '{label}, {current}');

  const roleOf = (item: MenuItem): string =>
    item.kind === 'radio' ? 'menuitemradio' : item.kind === 'check' ? 'menuitemcheckbox' : 'menuitem';

  function triggers(): HTMLButtonElement[] {
    return root ? [...root.querySelectorAll<HTMLButtonElement>('button.mb-trigger')] : [];
  }

  /** The items of one list (a menu or a submenu) that can take the focus, without those of a submenu inside it. */
  function itemsIn(list: Element | null | undefined): HTMLElement[] {
    if (!list) return [];
    return [...list.querySelectorAll<HTMLElement>('.mb-item:not(:disabled)')].filter(
      (el) => el.closest('[role="menu"]') === list,
    );
  }

  function itemButtons(id: string): HTMLElement[] {
    return itemsIn(root?.querySelector(`[data-menu="${id}"]`));
  }

  // Found by comparing the data, not by a selector: a key holds the id of a menu, which may hold any character.
  const subList = (key: string): HTMLElement | undefined =>
    root ? [...root.querySelectorAll<HTMLElement>('[data-sub]')].find((el) => el.dataset.sub === key) : undefined;
  const subParent = (key: string): HTMLElement | undefined =>
    root
      ? [...root.querySelectorAll<HTMLElement>('[data-sub-for]')].find((el) => el.dataset.subFor === key)
      : undefined;

  function clearSubTimers(): void {
    clearTimeout(openTimer);
    clearTimeout(graceTimer);
    openTimer = undefined;
    graceTimer = undefined;
    openTimerKey = null;
  }

  async function openSub(key: string, focusItem: 'first' | 'last' | null): Promise<void> {
    clearSubTimers();
    // Read now too: the resize event of a window that just changed may still be on its way.
    vw = viewWidth();
    subOpen = key;
    if (focusItem === null) return;
    await tick();
    const items = itemsIn(subList(key));
    (focusItem === 'first' ? items[0] : items[items.length - 1])?.focus();
  }

  function closeSub(focusParent: boolean): void {
    const was = subOpen;
    clearSubTimers();
    subOpen = null;
    if (focusParent && was !== null) subParent(was)?.focus();
  }

  function onParentClick(event: MouseEvent, key: string): void {
    // A click of the pointer folds an open list shut again in a narrow window; beside the menu it only opens (the
    // pointer may have opened it already by resting). Enter and Space are the keys' (`onItemKey`).
    if (narrow && subOpen === key && event.detail > 0) closeSub(false);
    else void openSub(key, null);
  }

  /** The focus on an item of a menu (not of a submenu) other than the open submenu's: the submenu beside it closes. */
  function onItemFocus(key: string, sub: string | null): void {
    if (sub === null && subOpen !== null && subOpen !== key && !narrow) closeSub(false);
  }

  /**
   * The pointer over an item of a menu: the item of a submenu opens it after a short rest; another item closes the
   * open submenu, unless the pointer crosses it on the way there: then it closes once the pointer rests that long
   * without arriving, or at once when it turns away.
   */
  function onItemPointer(event: PointerEvent, item: MenuItem, key: string, sub: string | null): void {
    if (sub !== null || event.pointerType === 'touch' || narrow) return;
    const point = { x: event.clientX, y: event.clientY };
    const before = last;
    last = point;
    const parent = item.kind === 'submenu' && submenuUsable(item);
    if (parent && subOpen === key) {
      clearTimeout(graceTimer);
      graceTimer = undefined;
      origin = point;
      return;
    }
    if (subOpen !== null) {
      const list = subList(subOpen);
      const side = list?.dataset.side === 'left' ? 'left' : 'right';
      const away = side === 'right' ? point.x < before.x : point.x > before.x;
      if (list && !away && inCorridor(origin, point, list.getBoundingClientRect(), side)) {
        clearTimeout(graceTimer);
        graceTimer = setTimeout(() => {
          graceTimer = undefined;
          closeSub(false);
        }, SUBMENU_GRACE_MS);
      } else {
        closeSub(false);
      }
    }
    if (parent && openTimerKey !== key && !(subOpen !== null && graceTimer !== undefined)) {
      clearTimeout(openTimer);
      openTimerKey = key;
      origin = point;
      openTimer = setTimeout(() => {
        openTimer = undefined;
        openTimerKey = null;
        void openSub(key, null);
      }, SUBMENU_OPEN_MS);
    }
  }

  function onParentLeave(key: string): void {
    if (openTimerKey !== key) return;
    clearTimeout(openTimer);
    openTimer = undefined;
    openTimerKey = null;
  }

  /**
   * Places a submenu beside its item (`flyoutPlace`), fixed to the window: once it is drawn, and again when the
   * window changes size or something scrolls under it.
   */
  function flyout(list: HTMLElement, key: string): { destroy: () => void } {
    if (canPopover) {
      try {
        list.showPopover();
      } catch {
        // stays a fixed box of the page
      }
    }
    const place = (): void => {
      const parent = subParent(key);
      if (!parent) return;
      list.style.left = '0px';
      list.style.top = '0px';
      // Where 0, 0 lands: a transformed ancestor (a menu moved into the window by `fit`) is the frame of a fixed box.
      const at0 = list.getBoundingClientRect();
      const style = getComputedStyle(list);
      const lift = list.clientTop + (parseFloat(style.paddingTop) || 0);
      const view = {
        width: document.documentElement.clientWidth || window.innerWidth,
        height: document.documentElement.clientHeight || window.innerHeight,
      };
      const at = flyoutPlace(parent.getBoundingClientRect(), at0, view, lift);
      list.style.left = `${at.left - at0.left}px`;
      list.style.top = `${at.top - at0.top}px`;
      list.dataset.side = at.side;
    };
    place();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return {
      destroy: () => {
        window.removeEventListener('resize', place);
        window.removeEventListener('scroll', place, true);
        try {
          if (list.matches(':popover-open')) list.hidePopover();
        } catch {
          // no popover
        }
      },
    };
  }

  onMount(() => () => clearSubTimers());

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
    if (openId !== id) {
      tell(id);
      closeSub(false);
    }
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
    // Moved by `left`, never by a transform: a transformed popup would be the frame of the fixed submenu inside it
    // (`flyout`), which it would then clip and scroll.
    const place = (): void => {
      popup.style.left = '';
      popup.style.right = '';
      const box = popup.getBoundingClientRect();
      const shift = popupShift(box.left, box.right, document.documentElement.clientWidth || window.innerWidth);
      if (shift !== 0) {
        popup.style.left = `${popup.offsetLeft + shift}px`;
        popup.style.right = 'auto';
      }
    };
    place();
    window.addEventListener('resize', place);
    return { destroy: () => window.removeEventListener('resize', place) };
  }

  function close(focusTrigger: boolean): void {
    const was = openId;
    closeSub(false);
    openId = null;
    if (focusTrigger && was) triggers().find((t) => t.dataset.for === was)?.focus();
  }

  function pick(item: MenuItem): void {
    if (item.kind === 'separator' || item.kind === 'info' || item.kind === 'section' || item.kind === 'submenu') return;
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
      default:
        // A typed letter goes to the next title that starts with it (ARIA menubar pattern); an open menu moves along.
        if (event.key.length === 1 && event.key !== ' ' && !event.ctrlKey && !event.metaKey && !event.altKey) {
          clearTimeout(typedTimer);
          typed += event.key;
          typedTimer = setTimeout(() => (typed = ''), TYPE_AHEAD_MS);
          const to = typeAheadIndex(
            list.map((t) => t.textContent?.trim() ?? ''),
            at,
            typed,
          );
          if (to >= 0) {
            event.preventDefault();
            const next = list[to];
            next?.focus();
            if (openId !== null && next?.dataset.for) void open(next.dataset.for, null);
          }
        }
    }
  }

  /**
   * The keys on an item of the menu `id`. `sub`: the key of the submenu the item is in (null in the menu itself);
   * `key`: the item's own place, which names its submenu when it has one.
   */
  function onItemKey(event: KeyboardEvent, id: string, item: MenuItem, sub: string | null, key: string): void {
    if (event.key === 'Enter' && apartKey(event) && pickApart(item)) {
      event.preventDefault();
      return;
    }
    if (item.kind === 'submenu' && (event.key === 'ArrowRight' || event.key === 'Enter' || event.key === ' ')) {
      event.preventDefault();
      if (submenuUsable(item)) void openSub(key, 'first');
      return;
    }
    if (sub !== null && event.key === 'ArrowLeft') {
      event.preventDefault();
      closeSub(true);
      return;
    }
    if (event.key === 'Escape' && (sub !== null || subOpen !== null)) {
      // The first Escape closes the submenu (back to its item), the next one the menu.
      event.preventDefault();
      event.stopPropagation();
      closeSub(sub !== null);
      return;
    }
    const self = event.currentTarget as HTMLElement;
    const items = itemsIn(self.closest('[role="menu"]'));
    const at = items.indexOf(self);
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
        // Closed here too: a bar of one menu opens the same menu again, which keeps what is open in it.
        closeSub(false);
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

{#snippet blocks(items: readonly MenuItem[], id: string, prefix: string, sub: string | null)}
  {#each toBlocks(items) as block, b (b)}
    {#if block.kind === 'divider'}
      <hr />
    {:else if block.heading !== null}
      <div role="group" aria-labelledby="{uid}-{prefix}-{b}">
        <p class="mb-section" id="{uid}-{prefix}-{b}" role="presentation">{block.heading}</p>
        {@render rows(block.items, id, `${prefix}/${b}`, sub)}
      </div>
    {:else}
      {@render rows(block.items, id, `${prefix}/${b}`, sub)}
    {/if}
  {/each}
{/snippet}

{#snippet rows(items: MenuItem[], id: string, prefix: string, sub: string | null)}
  {#each items as item, i (i)}
    {@const key = `${prefix}/${i}`}
    {#if item.kind === 'info'}
      <p class="mb-note" role="none">{item.label}</p>
    {:else if item.kind === 'submenu'}
      {@const usable = submenuUsable(item)}
      {@const current = submenuCurrent(item)}
      {@const expanded = subOpen === key}
      <!-- The item and its list side by side under one node without a role, as in the ARIA menu pattern. -->
      <div class="mb-sub" role="none">
        <button
          type="button"
          class="mb-item mb-parent"
          role="menuitem"
          aria-haspopup="menu"
          aria-expanded={expanded}
          aria-label={current !== null ? fill(currentText, { label: item.label, current }) : undefined}
          data-sub-for={key}
          disabled={!usable}
          tabindex="-1"
          use:tooltip={expanded ? '' : (item.hint ?? '')}
          onclick={(e: MouseEvent) => onParentClick(e, key)}
          onfocus={() => onItemFocus(key, sub)}
          onpointermove={(e: PointerEvent) => onItemPointer(e, item, key, sub)}
          onpointerleave={() => onParentLeave(key)}
          onkeydown={(e: KeyboardEvent) => onItemKey(e, id, item, sub, key)}
        >
          <span class="mb-mark" aria-hidden="true">{current !== null ? '●' : ''}</span>
          <span class="mb-text">{item.label}</span>
          <span class="mb-arrow" aria-hidden="true">{narrow && expanded ? '▾' : '▸'}</span>
        </button>
        {#if expanded && narrow}
          <div class="mb-subpopup mb-fold" role="menu" tabindex="-1" aria-label={item.label} data-sub={key}>
            {@render blocks(subItems(item), id, key, key)}
          </div>
        {:else if expanded}
          <div
            class="mb-subpopup mb-fly"
            role="menu"
            tabindex="-1"
            aria-label={item.label}
            data-sub={key}
            popover={canPopover ? 'manual' : undefined}
            use:flyout={key}
            onpointerenter={() => {
              clearTimeout(graceTimer);
              graceTimer = undefined;
            }}
          >
            {@render blocks(subItems(item), id, key, key)}
          </div>
        {/if}
      </div>
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
        onfocus={() => onItemFocus(key, sub)}
        onpointermove={(e: PointerEvent) => onItemPointer(e, item, key, sub)}
        onkeydown={(e: KeyboardEvent) => onItemKey(e, id, item, sub, key)}
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

<svelte:window onclick={onWindowClick} onkeydown={onWindowKey} onresize={() => (vw = viewWidth())} />

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
          {@render blocks(menu.items, menu.id, menu.id, null)}
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
  .mb-arrow {
    margin-left: 1.5rem;
    color: var(--muted);
  }
  .mb-item:hover:not(:disabled) .mb-arrow,
  .mb-item:focus-visible .mb-arrow {
    color: inherit;
  }
  /* The item of an open submenu stays marked while the keys or the pointer are in its list. */
  .mb-parent[aria-expanded='true']:not(:hover):not(:focus-visible) {
    background: var(--surface-2);
  }
  /* Beside the menu: fixed to the window and placed by `flyout`; a popover in the top layer where there is one
     (its own placement of the browser, centred with a border, is taken back). */
  .mb-fly {
    position: fixed;
    z-index: 21;
    min-width: 11rem;
    max-height: calc(100vh - 1rem);
    overflow-y: auto;
    box-sizing: border-box;
    margin: 0;
    inset: auto;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    padding: 0.3rem;
  }
  /* Folded open inside the menu (a narrow window): indented under its item; long labels wrap rather than make the
     menu scroll sideways. */
  .mb-fold {
    padding-left: 1rem;
  }
  .mb-fold .mb-item {
    white-space: normal;
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
