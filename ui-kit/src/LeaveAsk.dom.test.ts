// @vitest-environment jsdom
// The question of a dialog with typed text: it takes the focus on "Keep editing", and gives it back to the field.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import LeaveAsk from './LeaveAsk.svelte';

let app: ReturnType<typeof mount> | undefined;
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
  document.body.innerHTML = '';
});

const words = { confirm: 'Discard what you entered?', discard: 'Discard', keep: 'Keep editing' };

describe('LeaveAsk', () => {
  it('says its question with the words it is given and puts the focus on the safe answer', async () => {
    app = mount(LeaveAsk, { target: document.body, props: { ...words, ondiscard: () => {}, onkeep: () => {} } });
    flushSync();
    await tick();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain('Discard what you entered?');
    expect(document.activeElement?.textContent).toBe('Keep editing');
  });

  it('gives the focus back to the field that had it when the person keeps editing', async () => {
    document.body.innerHTML = '<input id="field">';
    const field = document.getElementById('field') as HTMLInputElement;
    field.focus();
    const calls: string[] = [];
    app = mount(LeaveAsk, {
      target: document.body,
      props: { ...words, ondiscard: () => calls.push('discard'), onkeep: () => calls.push('keep') },
    });
    flushSync();
    await tick();
    expect(document.activeElement).not.toBe(field);
    (document.activeElement as HTMLButtonElement).click();
    expect(calls).toEqual(['keep']);
    expect(document.activeElement).toBe(field);
    document.querySelectorAll('button')[0]?.click();
    expect(calls).toEqual(['keep', 'discard']);
  });
});
