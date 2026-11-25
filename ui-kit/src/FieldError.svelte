<script lang="ts">
  // What is wrong with a field, beside it. The field points here with `aria-describedby` and says `aria-invalid`; the
  // element is there only while the field is wrong.
  let { id, text }: { id: string; text: string | null | undefined } = $props();
</script>

{#if text}<span {id} class="field-error">{text}</span>{/if}
