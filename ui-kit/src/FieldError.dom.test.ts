// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import FieldError from './FieldError.svelte';

let app: ReturnType<typeof mount> | undefined;
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
  document.body.innerHTML = '';
});

describe('FieldError', () => {
  it('is there with its id while the field is wrong, and not otherwise', () => {
    app = mount(FieldError, { target: document.body, props: { id: 'name-error', text: 'Give it a name' } });
    flushSync();
    const span = document.getElementById('name-error');
    expect(span?.textContent).toBe('Give it a name');
    expect(span?.classList.contains('field-error')).toBe(true);
  });

  it('draws nothing for an empty text', () => {
    app = mount(FieldError, { target: document.body, props: { id: 'x', text: null } });
    flushSync();
    expect(document.getElementById('x')).toBeNull();
  });
});
