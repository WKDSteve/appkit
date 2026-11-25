// The send of an incomplete form puts the focus on the first field that says it is invalid.

import { describe, expect, it } from 'vitest';
import { focusFirstInvalid } from './formErrors';

function root(found: { focus: () => void } | null, seen: string[] = []): ParentNode {
  return {
    querySelector: (selector: string) => {
      seen.push(selector);
      return found;
    },
  } as unknown as ParentNode;
}

describe('focusFirstInvalid', () => {
  it('focuses the first invalid field and says so', () => {
    const focused: string[] = [];
    const seen: string[] = [];
    expect(focusFirstInvalid(root({ focus: () => focused.push('field') }, seen))).toBe(true);
    expect(focused).toEqual(['field']);
    expect(seen).toEqual(['[aria-invalid="true"]']);
  });

  it('does nothing when no field is invalid or there is no form', () => {
    expect(focusFirstInvalid(root(null))).toBe(false);
    expect(focusFirstInvalid(null)).toBe(false);
    expect(focusFirstInvalid(undefined)).toBe(false);
  });
});
