import { describe, expect, it } from 'vitest';
import base from './base.css?raw';

// A field is never smaller than 16 px (FORM-05): the rule for input, select and textarea comes after the one that
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
});
