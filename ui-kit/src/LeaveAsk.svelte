<script lang="ts">
  // The question of a dialog that holds something typed: discard it, or go on. Shown by `leaveGuard` when the person
  // pressed Escape; the focus goes to "Keep editing", the safe answer. The words are the app's (`kit.leave.*` of
  // `kitTexts` has them).
  import { onMount } from 'svelte';

  let {
    confirm,
    discard,
    keep: keepLabel,
    ondiscard,
    onkeep,
  }: { confirm: string; discard: string; keep: string; ondiscard: () => void; onkeep: () => void } = $props();
  let keep: HTMLButtonElement | undefined = $state();
  // The field that had the focus gets it back on "Keep editing": Escape must reach the dialog again.
  const opener = typeof document === 'undefined' ? null : (document.activeElement as HTMLElement | null);
  onMount(() => keep?.focus());
  const keepEditing = (): void => {
    onkeep();
    if (opener?.isConnected) opener.focus();
  };
</script>

<p class="leave-ask" role="alert">
  {confirm}
  <button type="button" onclick={ondiscard}>{discard}</button>
  <button type="button" class="primary" bind:this={keep} onclick={keepEditing}>{keepLabel}</button>
</p>

<style>
  .leave-ask {
    margin: 0 0 0.5rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--surface-2);
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 0.6rem;
    align-items: center;
  }
  button {
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.15rem 0.6rem;
    min-block-size: 24px;
    cursor: pointer;
  }
  button.primary {
    background: var(--accent);
    color: var(--accent-text);
    border-color: var(--accent);
  }
  @media (pointer: coarse) {
    button {
      min-block-size: 44px;
    }
  }
</style>
