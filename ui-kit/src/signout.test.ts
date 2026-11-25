import { describe, expect, it } from 'vitest';
import { signOut } from './signout';

describe('signOut', () => {
  it('asks the hub first and forgets the session after the answer', async () => {
    const order: string[] = [];
    await signOut(
      async () => void order.push('logout'),
      () => void order.push('forget'),
    );
    expect(order).toEqual(['logout', 'forget']);
  });

  it('forgets the session when the hub does not answer', async () => {
    let forgotten = 0;
    await signOut(
      () => Promise.reject(new Error('down')),
      () => void (forgotten += 1),
    );
    expect(forgotten).toBe(1);
  });

  it('forgets the session when asking the hub throws at once', async () => {
    let forgotten = 0;
    await signOut(
      () => {
        throw new Error('no client');
      },
      () => void (forgotten += 1),
    );
    expect(forgotten).toBe(1);
  });

  it('does not forget before the hub answered', async () => {
    let answer!: () => void;
    let forgotten = false;
    const done = signOut(
      () => new Promise<void>((r) => (answer = r)),
      () => void (forgotten = true),
    );
    await Promise.resolve();
    expect(forgotten).toBe(false);
    answer();
    await done;
    expect(forgotten).toBe(true);
  });
});
