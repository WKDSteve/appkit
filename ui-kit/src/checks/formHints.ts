// The check behind the `formHints` test of every app: every field a person types into or picks from explains itself.
// Hovering it (or its label), focusing it or tapping its label shows the hint popup of `appkit-ui/tooltip`. The popup
// looks for the closest `[data-hint]` around the element that has the focus, so the hint must sit on the field or on an
// element around it (its `<label>`): a hint on a sibling `<span>` shows on hover but never on focus. An app reads the
// components of its page (`import.meta.glob('./**/*.svelte', { query: '?raw', ... })`) and names each field without one.

import { parse } from 'svelte/compiler';

/** The elements that take an entry. */
const FIELDS = new Set(['input', 'textarea', 'select']);
/** Components that are fields and carry their hint by a prop of their own (it shows on their button). */
const FIELD_COMPONENTS: Record<string, string> = { MultiSelect: 'hint', PickField: 'hint', PickList: 'hint' };

interface Attr {
  type: string;
  name: string;
  value?: true | { type: string; data?: string }[] | { type: string };
}
interface Node {
  type: string;
  name?: string;
  start: number;
  attributes?: Attr[];
  [key: string]: unknown;
}

const attr = (n: Node, name: string): Attr | undefined => n.attributes?.find((a) => a.type === 'Attribute' && a.name === name);

/** A static text value of an attribute (`type="text"`), else undefined. */
function text(a: Attr | undefined): string | undefined {
  if (!a || !Array.isArray(a.value)) return undefined;
  const [only, ...more] = a.value;
  return only?.type === 'Text' && more.length === 0 ? only.data : undefined;
}

/**
 * The element carries a hint: `data-hint=...` or `use:tooltip`. Only an element of the page does: on a component
 * `data-hint` is a prop, which need not end up around the field.
 */
const hinted = (n: Node): boolean =>
  n.type === 'RegularElement' &&
  (n.attributes ?? []).some((a) => (a.type === 'Attribute' && a.name === 'data-hint') || (a.type === 'UseDirective' && a.name === 'tooltip'));

/**
 * What is no entry: a hidden input, a file input (an invisible picker opened by a button that has the hint), and a
 * field that is always read-only (`readonly` without a value: text to copy, not to type).
 */
function exempt(n: Node): boolean {
  const type = text(attr(n, 'type'));
  if (type === 'hidden' || type === 'file') return true;
  return attr(n, 'readonly')?.value === true;
}

export interface Missing {
  file: string;
  line: number;
  field: string;
}

/** The fields of one component without a hint on them or around them. */
export function fieldsWithoutHint(source: string, file: string): Missing[] {
  const out: Missing[] = [];
  const lineOf = (pos: number) => source.slice(0, pos).split('\n').length;
  function walk(value: unknown, around: Node[]): void {
    if (!value || typeof value !== 'object') return;
    if (Array.isArray(value)) {
      for (const v of value) walk(v, around);
      return;
    }
    const n = value as Node;
    let chain = around;
    if (n.type === 'RegularElement' || n.type === 'Component') {
      chain = [...around, n];
      const name = n.name ?? '';
      if (n.type === 'RegularElement' && FIELDS.has(name) && !exempt(n) && !chain.some(hinted)) {
        const type = text(attr(n, 'type'));
        out.push({ file, line: lineOf(n.start), field: type ? `${name}[type=${type}]` : name });
      }
      const prop = FIELD_COMPONENTS[name];
      if (n.type === 'Component' && prop !== undefined && !attr(n, prop) && !chain.some(hinted)) {
        out.push({ file, line: lineOf(n.start), field: name });
      }
    }
    for (const [key, child] of Object.entries(n)) {
      if (key === 'attributes' || key === 'metadata' || key === 'parent') continue;
      walk(child, chain);
    }
  }
  walk((parse(source, { modern: true }) as unknown as { fragment: unknown }).fragment, []);
  return out;
}
