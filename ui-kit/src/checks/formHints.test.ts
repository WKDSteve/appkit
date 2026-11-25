import { describe, expect, it } from 'vitest';
import { fieldsWithoutHint } from './formHints';

describe('fieldsWithoutHint', () => {
  it('finds a field without one, and not one with a hint on its label', () => {
    const src = [
      '<label><span>Title</span><input type="text" /></label>',
      '<label data-hint="The name."><span>Name</span><input /></label>',
      '<label><span data-hint="On the span only">Prio</span><select></select></label>',
      '{#if x}<textarea data-hint="Body"></textarea>{/if}',
      '<input type="file" hidden /><input type="hidden" /><textarea readonly></textarea>',
      '<MultiSelect hint="Areas" /><MultiSelect />',
      '{#snippet row()}<input use:tooltip={"x"} /><input readonly={busy} />{/snippet}',
      '<Card data-hint="A prop, not an element"><input type="date" /></Card>',
    ].join('\n');
    expect(fieldsWithoutHint(src, 'T.svelte').map((m) => `${m.line}:${m.field}`)).toEqual([
      '1:input[type=text]',
      '3:select',
      '6:MultiSelect',
      '7:input',
      '8:input[type=date]',
    ]);
  });
});
