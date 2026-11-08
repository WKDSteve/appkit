<script lang="ts">
  // A choice of one value that can be taken away in one click: the select of the browser with an x inside its
  // border while a value other than the empty one is chosen. The options are the app's (`children`), and so is the
  // label around it; every other attribute (onchange, aria-label, data-*, disabled) goes to the select.
  import type { Snippet } from 'svelte';
  import type { HTMLSelectAttributes } from 'svelte/elements';
  import { tooltip } from './tooltip';

  let {
    value,
    clearLabel,
    onclear,
    empty = '',
    children,
    ...rest
  }: {
    /** The chosen value. */
    value: string;
    /** The words of the x for a screen reader and the hint popup ("Remove the selection Updated", `kit.select.clear`). */
    clearLabel: string;
    /** Takes the choice away: the app sets the value back to `empty` (and with it the address or the saved view). */
    onclear: () => void;
    /** The value that means "nothing chosen"; the x shows for every other one. */
    empty?: string;
    children: Snippet;
  } & Omit<HTMLSelectAttributes, 'value' | 'children'> = $props();

  let select: HTMLSelectElement | undefined = $state();
  const chosen = $derived(value !== empty);

  // The focus goes back to the select: the x it was on is gone.
  function clear(event: Event): void {
    event.preventDefault();
    event.stopPropagation();
    onclear();
    select?.focus();
  }

  // Delete or Backspace on the select take the choice away too (Enter and Space open the list of the browser).
  function onKey(event: KeyboardEvent): void {
    if ((event.key === 'Delete' || event.key === 'Backspace') && chosen && !rest.disabled) {
      event.preventDefault();
      onclear();
    }
  }
</script>

<span class="clear-select">
  <select bind:this={select} {value} class:clearable={chosen} onkeydown={onKey} {...rest}>
    {@render children()}
  </select>
  {#if chosen && !rest.disabled}
    <!-- Beside the select in the DOM, drawn inside its border; no empty spot while nothing is chosen. -->
    <button
      type="button"
      class="clear"
      aria-label={clearLabel}
      use:tooltip={clearLabel}
      onmousedown={(e) => e.preventDefault()}
      onclick={clear}
    >
      <span aria-hidden="true">×</span>
    </button>
  {/if}
</span>

<style>
  .clear-select {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  /* Room for the x before the arrow of the browser. */
  select.clearable {
    padding-right: 2.6rem;
  }
  .clear {
    position: absolute;
    top: 50%;
    right: 1.4rem;
    transform: translateY(-50%);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.2rem;
    height: 1.2rem;
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
    background: var(--surface-2);
    color: var(--text);
  }
</style>
