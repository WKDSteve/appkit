// A page with something typed asks before the tab is left, and stops asking when it is told to.

import { describe, expect, it } from 'vitest';
import { warnBeforeUnload } from './unsaved';

describe('warnBeforeUnload', () => {
  it('cancels the unload event while it is on, and not after it was taken away', () => {
    const target = new EventTarget();
    const unload = (): Event => {
      const event = new Event('beforeunload', { cancelable: true });
      target.dispatchEvent(event);
      return event;
    };
    const off = warnBeforeUnload(target);
    const asked = unload();
    expect(asked.defaultPrevented).toBe(true);
    expect((asked as Event & { returnValue?: unknown }).returnValue).toBe(false);
    off();
    expect(unload().defaultPrevented).toBe(false);
  });
});
