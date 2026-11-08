// @vitest-environment jsdom
// The choice of one value with its x: the x shows only while a value is chosen, a click or Delete empties it, the
// focus goes back to the select, and the attributes of the app reach the select.
import { createRawSnippet, flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import ClearSelect from './ClearSelect.svelte';

let app: ReturnType<typeof mount> | undefined;

// A raw snippet has one root: the options come in one group.
const options = createRawSnippet(() => ({
  render: () => '<optgroup label="When"><option value="">Any time</option><option value="week">This week</option></optgroup>',
}));

function build(extra: Record<string, unknown> = {}): { clears: number } {
  const counter = { clears: 0 };
  app = mount(ClearSelect, {
    target: document.body,
    props: {
      value: '',
      clearLabel: 'Remove the selection Updated',
      onclear: () => (counter.clears += 1),
      children: options,
      ...extra,
    },
  });
  flushSync();
  return counter;
}
beforeEach(() => {
  document.body.innerHTML = '';
});
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
});

const select = () => document.querySelector<HTMLSelectElement>('select')!;
const clearButton = () => document.querySelector<HTMLButtonElement>('button.clear');

describe('the x of a choice of one value', () => {
  it('shows no x for the empty value', () => {
    build();
    expect(clearButton()).toBeNull();
    expect(select().classList.contains('clearable')).toBe(false);
  });

  it('shows a named x for a chosen value, and a click empties it and puts the focus on the select', async () => {
    const counter = build({ value: 'week' });
    expect(select().value).toBe('week');
    const x = clearButton()!;
    expect(x.getAttribute('aria-label')).toBe('Remove the selection Updated');
    x.click();
    await tick();
    expect(counter.clears).toBe(1);
    expect(document.activeElement).toBe(select());
  });

  it('takes the choice away with Delete or Backspace on the select', () => {
    const counter = build({ value: 'week' });
    for (const key of ['Delete', 'Backspace']) {
      const press = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      select().dispatchEvent(press);
      expect(press.defaultPrevented).toBe(true);
    }
    expect(counter.clears).toBe(2);
  });

  it('leaves Delete alone while nothing is chosen', () => {
    const counter = build();
    const press = new KeyboardEvent('keydown', { key: 'Delete', bubbles: true, cancelable: true });
    select().dispatchEvent(press);
    expect(press.defaultPrevented).toBe(false);
    expect(counter.clears).toBe(0);
  });

  it('knows another empty value', () => {
    build({ value: 'any', empty: 'any' });
    expect(clearButton()).toBeNull();
  });

  it('has no x while the select is disabled', () => {
    build({ value: 'week', disabled: true });
    expect(clearButton()).toBeNull();
  });

  it('hands the attributes of the app to the select', () => {
    const changes: string[] = [];
    build({ value: 'week', 'data-range': 'prio-min', 'aria-label': 'Updated', onchange: (e: Event) => changes.push((e.currentTarget as HTMLSelectElement).value) });
    expect(select().dataset['range']).toBe('prio-min');
    expect(select().getAttribute('aria-label')).toBe('Updated');
    select().value = '';
    select().dispatchEvent(new Event('change', { bubbles: true }));
    expect(changes).toEqual(['']);
  });
});
