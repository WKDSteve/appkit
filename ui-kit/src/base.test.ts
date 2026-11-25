import { describe, expect, it } from 'vitest';
import base from './base.css?raw';
import docsDialog from './DocsDialog.svelte?raw';
import palette from './Palette.svelte?raw';

// A field is never smaller than 16 px: the rule for input, select and textarea comes after the one that
// lets them inherit the size of the text, and it names the three of them.
describe('base.css: the size of a field', () => {
  it('sets at least 1rem for input, select and textarea, after the inherit rule', () => {
    const inherit = base.indexOf('font: inherit;');
    const rule = base.search(/input,\s*select,\s*textarea\s*\{[^}]*font-size:\s*max\(1rem,/);
    expect(inherit).toBeGreaterThan(-1);
    expect(rule).toBeGreaterThan(inherit);
  });

  it('lets textarea inherit the font of the page like the other fields', () => {
    expect(base).toMatch(/button,\s*input,\s*select,\s*textarea\s*\{[^}]*font:\s*inherit;/);
  });

  it('balances the headings and gives compared numbers one width', () => {
    expect(base).toMatch(/h1,\s*h2,\s*h3\s*\{[^}]*text-wrap:\s*balance;/);
    expect(base).toMatch(/table,\s*\.num,\s*\.count\s*\{[^}]*font-variant-numeric:\s*tabular-nums;/);
  });
});

// A component that sets its own `font: inherit` on a field must size it again, or the field falls back to the size
// of the text (14 px) and iOS Safari zooms into the page when it takes the focus.
describe('components with their own field style', () => {
  for (const [name, source] of [
    ['Palette', palette],
    ['DocsDialog', docsDialog],
  ] as const) {
    it(`${name} keeps its input at 1rem or more, after the inherit`, () => {
      const rule = /\n {2}input \{([^}]*)\}/.exec(source);
      expect(rule).not.toBeNull();
      const body = rule?.[1] ?? '';
      expect(body).toMatch(/font-size:\s*max\(1rem,\s*var\(--font-size\)\)/);
      expect(body.indexOf('font-size')).toBeGreaterThan(body.indexOf('font: inherit'));
    });
  }
});

describe('base.css: the info button', () => {
  it('is at least 24 px, and 44 px for a finger', () => {
    const rule = /\n\.info \{([^}]*)\}/.exec(base);
    const body = rule?.[1] ?? '';
    expect(body).toMatch(/min-inline-size:\s*24px/);
    expect(body).toMatch(/min-block-size:\s*24px/);
    expect(base).toMatch(
      /@media \(pointer: coarse\) \{\s*\.info \{[^}]*min-inline-size:\s*44px;[^}]*min-block-size:\s*44px/,
    );
  });
});
