// The markup of the groups, rendered on the server side of Svelte: columns and sections, the counts, the folding, the
// keys. The drag itself is looked at in the browser tests of the views.
import { createRawSnippet } from 'svelte';
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import type { Group } from './groups';
import Groups from './Groups.svelte';

// An item of any app: the component reads only its id.
type Item = { id: string; area: string };

const task = (id: string): Item => ({ id, area: 'a' });
const groups: Group<Item>[] = [
  { value: 'doing', items: [task('a/one'), task('a/two')] },
  { value: 'open', items: [] },
  { value: '', items: [task('a/three')] },
];
const item = createRawSnippet((t: () => Item) => ({ render: () => `<span class="card">${t().id}</span>` }));
const html = (props: Record<string, unknown>) =>
  render(Groups, {
    props: { groups, label: (v: string) => (v === '' ? 'none' : v.toUpperCase()), item, columnsLabel: 'Columns', ...props } as never,
  }).body.replace(/<!--.*?-->/g, '');

describe('the groups as columns', () => {
  it('draws a column for every group, the empty one too, each named with its count', () => {
    const out = html({ mode: 'columns' });
    expect(out).toContain('aria-label="Columns"');
    expect(out).toContain('aria-label="DOING (2)"');
    expect(out).toContain('aria-label="OPEN (0)"');
    expect(out).toContain('aria-label="none (1)"');
    expect(out.match(/data-group=/g)).toHaveLength(3);
    expect(out.match(/class="card"/g)).toHaveLength(3);
  });

  it('is not draggable without a handler for the drop, and is with one', () => {
    expect(html({ mode: 'columns' })).not.toContain('draggable="true"');
    expect(html({ mode: 'columns', ondrop: () => undefined })).toContain('draggable="true"');
    // While a write is under way nothing is dragged.
    expect(html({ mode: 'columns', ondrop: () => undefined, busy: true })).not.toContain('draggable="true"');
  });

  it('puts the explanation of a group on its heading', () => {
    expect(html({ mode: 'columns', hint: (v: string) => `about ${v}` })).toContain('data-hint="about doing"');
  });
});

describe('the groups as a list', () => {
  it('draws a section for every group with a button that says whether it is open, and the tasks of an open one', () => {
    const out = html({ mode: 'list' });
    expect(out.match(/aria-expanded="true"/g)).toHaveLength(3);
    expect(out).toContain('data-item="a/one"');
    expect(out).toContain('data-item="a/three"');
  });

  it('folds the groups it is told to: the button says so and the tasks are not drawn', () => {
    const out = html({ mode: 'list', collapsed: ['doing'] });
    expect(out.match(/aria-expanded="false"/g)).toHaveLength(1);
    expect(out).not.toContain('data-item="a/one"');
    expect(out).toContain('data-item="a/three"');
    // The count stays: a folded group says how much it holds.
    expect(out).toContain('aria-label="DOING (2)"');
  });
});
