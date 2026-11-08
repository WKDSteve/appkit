<script lang="ts">
  // A choice of any number of values. The name of the choice is the label of
  // the button, never one of the options.
  import { tooltip } from './tooltip';

  export interface Option {
    value: string;
    label: string;
    /** More words for the option (a hint, or the real name behind a label). */
    title?: string;
    /** Shown after the label, in brackets; not searched (a count would make the search find digits). */
    suffix?: string;
  }

  let {
    label,
    options,
    selected,
    onchange,
    resetLabel,
    searchLabel,
    hint = '',
    note = '',
    clearLabel = '',
    match,
  }: {
    label: string;
    options: readonly Option[];
    selected: readonly string[];
    onchange: (next: string[]) => void;
    resetLabel: string;
    searchLabel: string;
    /** What the choice is for: the hint popup shows it on the button (not while the choice is open). */
    hint?: string;
    /** One line at the top of the open popup, e.g. how the chosen values combine ("any of these"). */
    note?: string;
    /**
     * The words of the x that shows on the button while something is chosen and takes the choice away in one click
     * ("Remove the selection Status", `kit.select.clear`). Left out, it is `resetLabel: label`.
     */
    clearLabel?: string;
    /**
     * A switch between "any of the chosen values" and "all of them", for a field a task can hold several values of
     * (tags). Left out, the choice has no switch; the app says in `note` how its values combine.
     */
    match?: {
      value: 'any' | 'all';
      anyLabel: string;
      allLabel: string;
      onchange: (value: 'any' | 'all') => void;
    };
  } = $props();

  // The radios of the any/all switch form one group per choice, never one across the page.
  const uid = $props.id();
  let open = $state(false);
  let needle = $state('');
  let root: HTMLElement | undefined = $state();
  let button: HTMLButtonElement | undefined = $state();
  // The width of the button when the popup opened. The label names what is chosen ("Areas: 2"), so it grows and
  // shrinks with every tick; the row around it would move and the popup, which hangs from the button, with it - the
  // next click would land beside the option it was aimed at. While the popup is open the button keeps this width.
  let openWidth = $state(0);

  // A list that long is worth a way to narrow it.
  const SEARCH_FROM = 10;
  // The search finds an option by its label, its value or its more words (a real name or an alias behind a label).
  const shownOptions = $derived.by(() => {
    const word = needle.trim().toLowerCase();
    if (word === '') return options;
    return options.filter((o) =>
      [o.label, o.value, o.title ?? ''].some((text) => text.toLowerCase().includes(word)),
    );
  });
  const summary = $derived.by(() => {
    if (selected.length === 0) return label;
    if (selected.length === 1) {
      const one = options.find((o) => o.value === selected[0]);
      return `${label}: ${one?.label ?? selected[0]}`;
    }
    return `${label}: ${selected.length}`;
  });

  // The x that empties the choice: named for a screen reader, never an empty spot while nothing is chosen.
  const clearName = $derived(clearLabel !== '' ? clearLabel : `${resetLabel}: ${label}`);

  // The x takes this choice away without opening the popup; the focus goes to the button, since the x is gone.
  function clear(event: Event): void {
    event.stopPropagation();
    if (open) close(false);
    onchange([]);
    button?.focus();
  }

  // Delete or Backspace on the closed button take the choice away too, for the keyboard (the x is also a tab stop).
  function onButtonKey(event: KeyboardEvent): void {
    if ((event.key === 'Delete' || event.key === 'Backspace') && !open && selected.length > 0) {
      event.preventDefault();
      onchange([]);
    }
  }

  function toggle(value: string): void {
    onchange(selected.includes(value) ? selected.filter((v) => v !== value) : [...selected, value]);
  }

  function openPopup(): void {
    openWidth = button?.getBoundingClientRect().width ?? 0;
    open = true;
  }

  function close(focusButton: boolean): void {
    open = false;
    openWidth = 0;
    needle = '';
    if (focusButton) button?.focus();
  }

  function onWindowClick(event: MouseEvent): void {
    if (open && root && event.target instanceof Node && !root.contains(event.target)) {
      close(false);
    }
  }

  // A press on a spot of the popup that takes no focus (its padding, the gap between the search and the list, the
  // edge of a row) would move the focus to the body and close the popup through focusout, with the person still in
  // the choice. The focus stays where it is; a field or a box still takes it, and a label still ticks its box.
  function onPopupMouseDown(event: MouseEvent): void {
    const target = event.target;
    if (!(target instanceof HTMLInputElement || target instanceof HTMLButtonElement)) event.preventDefault();
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key === 'Escape' && open) {
      // The page's own Escape (leave the selection) must not also fire, nor the dialog's own (it would close).
      event.stopPropagation();
      event.preventDefault();
      close(true);
    }
  }
</script>

<svelte:window onclick={onWindowClick} />

<!-- Focus that leaves the choice (Tab) closes the popup: it must not stay open behind the next control. -->
<div
  class="multi"
  bind:this={root}
  onkeydown={onKey}
  onfocusout={(e) => {
    const to = e.relatedTarget;
    // The window lost the focus (another window, Alt+Tab): the choice is still where the person left it.
    if (to === null && !document.hasFocus()) return;
    if (open && root && !(to instanceof Node && root.contains(to))) close(false);
  }}
  role="presentation"
>
  <button
    bind:this={button}
    type="button"
    class="trigger"
    class:active={selected.length > 0}
    class:clearable={selected.length > 0}
    aria-haspopup="true"
    aria-expanded={open}
    class:held={open && openWidth > 0}
    style:width={open && openWidth > 0 ? `${openWidth}px` : undefined}
    use:tooltip={open ? '' : hint}
    onclick={() => (open ? close(false) : openPopup())}
    onkeydown={onButtonKey}
  >
    <span class="summary">{summary}</span>
    <span class="caret" aria-hidden="true">▾</span>
  </button>
  {#if selected.length > 0}
    <!-- Beside the button in the DOM (a button holds no button), drawn inside its border. -->
    <button
      type="button"
      class="clear"
      aria-label={clearName}
      use:tooltip={clearName}
      onmousedown={(e) => e.preventDefault()}
      onclick={clear}
    >
      <span aria-hidden="true">×</span>
    </button>
  {/if}
  {#if open}
    <!-- The listener steers the focus only (see onPopupMouseDown); the group itself is not interactive. -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div class="popup" role="group" aria-label={label} onmousedown={onPopupMouseDown}>
      {#if note !== ''}<p class="note">{note}</p>{/if}
      {#if match}
        <div class="match" role="radiogroup" aria-label={label}>
          <label>
            <input
              type="radio"
              name={`match-${uid}`}
              value="any"
              checked={match.value === 'any'}
              onchange={() => match.onchange('any')}
            />
            <span>{match.anyLabel}</span>
          </label>
          <label>
            <input
              type="radio"
              name={`match-${uid}`}
              value="all"
              checked={match.value === 'all'}
              onchange={() => match.onchange('all')}
            />
            <span>{match.allLabel}</span>
          </label>
        </div>
      {/if}
      {#if options.length >= SEARCH_FROM}
        <input
          type="search"
          class="narrow"
          placeholder={searchLabel}
          aria-label={searchLabel}
          bind:value={needle}
        />
      {/if}
      <ul>
        {#each shownOptions as option (option.value)}
          <li>
            <label title={option.title}>
              <input
                type="checkbox"
                checked={selected.includes(option.value)}
                onchange={() => toggle(option.value)}
              />
              <span>{option.label}{option.suffix === undefined ? '' : ` (${option.suffix})`}</span>
            </label>
          </li>
        {/each}
      </ul>
      {#if selected.length > 0}
        <button type="button" class="reset" onclick={() => onchange([])}>{resetLabel}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .multi {
    position: relative;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.3rem 0.6rem;
    cursor: pointer;
    color: var(--text);
  }
  .trigger.active {
    border-color: var(--accent);
  }
  /* Room for the x, which sits inside the border on the right (see .clear). */
  .trigger.clearable {
    padding-right: 1.9rem;
  }
  .clear {
    position: absolute;
    top: 50%;
    right: 0.3rem;
    transform: translateY(-50%);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.3rem;
    height: 1.3rem;
    padding: 0;
    border: none;
    border-radius: var(--radius);
    background: none;
    color: var(--muted);
    cursor: pointer;
    line-height: 1;
  }
  .clear:hover,
  .clear:focus-visible {
    background: var(--surface);
    color: var(--text);
  }
  /* A held button cuts its label instead of growing with it (see openWidth). */
  .trigger.held {
    box-sizing: border-box;
  }
  .trigger.held .summary {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .caret {
    color: var(--muted);
    font-size: 0.8em;
  }
  .note {
    margin: 0 0 0.3rem;
    color: var(--muted);
    font-size: 0.85em;
  }
  .match {
    display: flex;
    gap: 0.8rem;
    margin-bottom: 0.3rem;
  }
  .match label {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    cursor: pointer;
  }
  .popup {
    position: absolute;
    z-index: 10;
    top: calc(100% + 0.25rem);
    left: 0;
    min-width: 14rem;
    max-width: 24rem;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    padding: 0.4rem;
  }
  .narrow {
    width: 100%;
    box-sizing: border-box;
    margin-bottom: 0.3rem;
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.25rem 0.4rem;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 20rem;
    overflow: auto;
  }
  li label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.2rem 0.3rem;
    border-radius: var(--radius);
    cursor: pointer;
  }
  /* A long option label wraps inside the popup instead of being cut. */
  li label span {
    overflow-wrap: anywhere;
  }
  li label:hover {
    background: var(--surface-2);
  }
  .reset {
    margin-top: 0.3rem;
    width: 100%;
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.25rem;
    cursor: pointer;
    color: var(--text);
  }
</style>
