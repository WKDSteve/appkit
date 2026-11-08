// @vitest-environment jsdom
// The choice itself: the button keeps its width while the popup is open, the search finds an option by more than its
// label, the note says how the values combine, and the any/all switch reports the side chosen.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import MultiSelect from './MultiSelect.svelte';

let app: ReturnType<typeof mount> | undefined;

const OPTIONS = Array.from({ length: 12 }, (_, i) => ({ value: `v${i}`, label: `Option ${i}` }));

function build(extra: Record<string, unknown> = {}, options: readonly { value: string; label: string; title?: string }[] = OPTIONS) {
  document.body.innerHTML = '';
  app = mount(MultiSelect, {
    target: document.body,
    props: {
      label: 'Areas',
      options,
      selected: [],
      onchange: () => undefined,
      resetLabel: 'Reset',
      searchLabel: 'Search',
      ...extra,
    },
  });
  flushSync();
}
beforeEach(() => {
  document.body.innerHTML = '';
});
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
});

const button = () => document.querySelector<HTMLButtonElement>('button.trigger')!;
const shown = () => [...document.querySelectorAll('.popup li span')].map((s) => s.textContent);

async function openIt(): Promise<void> {
  button().click();
  await tick();
  await tick();
}

async function search(word: string): Promise<void> {
  const input = document.querySelector<HTMLInputElement>('.popup input[type=search]')!;
  input.value = word;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await tick();
}

describe('the button of a choice', () => {
  it('keeps the width it had when the popup opened, and lets it go when the popup closes', async () => {
    build();
    vi.spyOn(button(), 'getBoundingClientRect').mockReturnValue({ width: 123 } as DOMRect);
    await openIt();
    expect(button().style.width).toBe('123px');
    expect(button().classList.contains('held')).toBe(true);
    await openIt();
    expect(button().style.width).toBe('');
    expect(button().classList.contains('held')).toBe(false);
  });
});

describe('the search of a choice', () => {
  it('finds an option by its value and by its more words, not only by its label', async () => {
    build({}, [
      ...OPTIONS,
      { value: 'wkdbook-tasks.nvim', label: 'tasks.nvim', title: 'also tasks-desktop' },
    ]);
    await openIt();
    await search('wkdbook');
    expect(shown()).toEqual(['tasks.nvim']);
    await search('tasks-desktop');
    expect(shown()).toEqual(['tasks.nvim']);
    await search('Option 1');
    expect(shown()).toEqual(['Option 1', 'Option 10', 'Option 11']);
  });
});

describe('the note and the any/all switch', () => {
  it('shows the note at the top of the open popup only', async () => {
    build({ note: 'Any of these' });
    expect(document.querySelector('.note')).toBeNull();
    await openIt();
    expect(document.querySelector('.popup .note')?.textContent).toBe('Any of these');
  });

  it('has no switch unless the app asks for one', async () => {
    build();
    await openIt();
    expect(document.querySelector('.match')).toBeNull();
  });

  it('shows the side chosen and reports a change', async () => {
    const changes: string[] = [];
    build({ match: { value: 'any', anyLabel: 'any', allLabel: 'all', onchange: (v: string) => changes.push(v) } });
    await openIt();
    const radios = [...document.querySelectorAll<HTMLInputElement>('.match input[type=radio]')];
    expect(radios.map((r) => r.checked)).toEqual([true, false]);
    expect(radios[0]!.name).toBe(radios[1]!.name);
    radios[1]!.click();
    expect(changes).toEqual(['all']);
  });
});

describe('the x of a choice', () => {
  const clearButton = () => document.querySelector<HTMLButtonElement>('button.clear');

  it('shares one box with the button, so it is placed against the button and not against the whole choice', () => {
    build({ selected: ['v1'] });
    const anchor = clearButton()!.parentElement!;
    expect(anchor.classList.contains('anchor')).toBe(true);
    expect(anchor.contains(button())).toBe(true);
    expect(anchor.parentElement!.classList.contains('multi')).toBe(true);
  });

  it('shows no x while nothing is chosen', () => {
    build();
    expect(clearButton()).toBeNull();
    expect(button().classList.contains('clearable')).toBe(false);
  });

  it('takes the whole choice away in one click, without opening the popup', async () => {
    const changes: string[][] = [];
    build({ selected: ['v1', 'v2'], clearLabel: 'Remove the selection Areas', onchange: (n: string[]) => changes.push(n) });
    const x = clearButton()!;
    expect(x.getAttribute('aria-label')).toBe('Remove the selection Areas');
    x.click();
    await tick();
    expect(changes).toEqual([[]]);
    expect(document.querySelector('.popup')).toBeNull();
    expect(document.activeElement).toBe(button());
  });

  it('closes an open popup when the x is clicked', async () => {
    build({ selected: ['v1'] });
    await openIt();
    expect(document.querySelector('.popup')).not.toBeNull();
    clearButton()!.click();
    await tick();
    expect(document.querySelector('.popup')).toBeNull();
  });

  it('is named by the reset words and the label when the app gives no words of its own', () => {
    build({ selected: ['v1'] });
    expect(clearButton()!.getAttribute('aria-label')).toBe('Reset: Areas');
  });

  it('keeps the focus where it is when the x is pressed (no focusout that closes the popup)', () => {
    build({ selected: ['v1'] });
    const down = new MouseEvent('mousedown', { bubbles: true, cancelable: true });
    clearButton()!.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
  });

  it('takes the choice away with Delete or Backspace on the closed button', () => {
    const changes: string[][] = [];
    build({ selected: ['v1'], onchange: (n: string[]) => changes.push(n) });
    for (const key of ['Delete', 'Backspace']) {
      const press = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      button().dispatchEvent(press);
      expect(press.defaultPrevented).toBe(true);
    }
    expect(changes).toEqual([[], []]);
  });

  it('leaves Delete alone while nothing is chosen', () => {
    const changes: string[][] = [];
    build({ onchange: (n: string[]) => changes.push(n) });
    const press = new KeyboardEvent('keydown', { key: 'Delete', bubbles: true, cancelable: true });
    button().dispatchEvent(press);
    expect(press.defaultPrevented).toBe(false);
    expect(changes).toEqual([]);
  });
});
