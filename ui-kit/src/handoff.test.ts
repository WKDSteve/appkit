import { describe, expect, it } from 'vitest';
import { askForSession, shareSession, type ChannelLike } from './handoff';

type Listener = (e: { data: unknown }) => void;

/** Channels of one origin: a message posted on one reaches every other one, never the one that posted it. */
function bus() {
  const all: Array<{ listeners: Set<Listener> }> = [];
  return (): ChannelLike & { sent: unknown[]; listening: () => number } => {
    const me = { listeners: new Set<Listener>() };
    all.push(me);
    const sent: unknown[] = [];
    return {
      sent,
      listening: () => me.listeners.size,
      postMessage(data) {
        sent.push(data);
        for (const other of all) if (other !== me) for (const l of [...other.listeners]) l({ data });
      },
      addEventListener: (_t, l) => void me.listeners.add(l),
      removeEventListener: (_t, l) => void me.listeners.delete(l),
    };
  };
}

function clock() {
  const timers: Array<{ run: () => void; cleared: boolean }> = [];
  return {
    setTimeout: (run: () => void) => {
      const t = { run, cleared: false };
      timers.push(t);
      return t;
    },
    clearTimeout: (h: unknown) => void ((h as { cleared: boolean }).cleared = true),
    fire: () => timers.filter((t) => !t.cleared).forEach((t) => t.run()),
    timers,
  };
}

const TOKEN = 'ab'.repeat(32);
const random = () => new Uint8Array(16).fill(7);
const NONCE = '07'.repeat(16);

describe('handoff of the session to a new tab', () => {
  it('a live tab with a session answers, and the new tab takes the token and stops listening', async () => {
    const make = bus();
    const old = make();
    const fresh = make();
    shareSession({ channel: old, token: () => TOKEN, live: () => true });
    const c = clock();
    const token = await askForSession({ channel: fresh, random, ...c });
    expect(token).toBe(TOKEN);
    expect(fresh.listening()).toBe(0);
    expect(c.timers[0]?.cleared).toBe(true);
  });

  it('a tab whose stream is not open, or that has no session, does not answer: the new tab pairs', async () => {
    const make = bus();
    shareSession({ channel: make(), token: () => TOKEN, live: () => false });
    shareSession({ channel: make(), token: () => null, live: () => true });
    const fresh = make();
    const c = clock();
    const asked = askForSession({ channel: fresh, random, ...c });
    c.fire();
    expect(await asked).toBeNull();
    expect(fresh.listening()).toBe(0);
  });

  it('takes no answer to another question, and no token that is not one', async () => {
    const make = bus();
    const liar = make();
    const fresh = make();
    const c = clock();
    const asked = askForSession({ channel: fresh, random, ...c });
    for (const data of [
      { appkitSession: 1, answer: 'ff'.repeat(16), token: TOKEN },
      { appkitSession: 1, answer: NONCE, token: 'short' },
      { appkitSession: 1, answer: NONCE, token: `${TOKEN}\n` },
      { appkitSession: 2, answer: NONCE, token: TOKEN },
      { answer: NONCE, token: TOKEN },
      null,
      'x',
    ]) {
      liar.postMessage(data);
    }
    c.fire();
    expect(await asked).toBeNull();
  });

  it('an answer after the wait reaches onLate once, and the promise has said null', async () => {
    const make = bus();
    const late = make();
    const fresh = make();
    const c = clock();
    const taken: string[] = [];
    const asked = askForSession({ channel: fresh, random, ...c, onLate: (t) => taken.push(t) });
    c.fire();
    expect(await asked).toBeNull();
    expect(fresh.listening()).toBe(1);
    // Not an answer to this question: it keeps waiting.
    late.postMessage({ appkitSession: 1, answer: 'ff'.repeat(16), token: TOKEN });
    expect(taken).toEqual([]);
    late.postMessage({ appkitSession: 1, answer: NONCE, token: TOKEN });
    expect(taken).toEqual([TOKEN]);
    expect(fresh.listening()).toBe(0);
    late.postMessage({ appkitSession: 1, answer: NONCE, token: 'cd'.repeat(32) });
    expect(taken).toEqual([TOKEN]);
  });

  it('an answer in time does not reach onLate', async () => {
    const make = bus();
    shareSession({ channel: make(), token: () => TOKEN, live: () => true });
    const fresh = make();
    const taken: string[] = [];
    const token = await askForSession({ channel: fresh, random, ...clock(), onLate: (t) => taken.push(t) });
    expect(token).toBe(TOKEN);
    expect(taken).toEqual([]);
    expect(fresh.listening()).toBe(0);
  });

  it('answers only a question that is one, and stops answering when stopped', () => {
    const make = bus();
    const old = make();
    const other = make();
    const stop = shareSession({ channel: old, token: () => TOKEN, live: () => true });
    other.postMessage({ appkitSession: 1, ask: 'not hex' });
    other.postMessage({ appkitSession: 1, ask: 'ab'.repeat(17) });
    expect(old.sent).toEqual([]);
    other.postMessage({ appkitSession: 1, ask: 'ab'.repeat(16) });
    expect(old.sent).toEqual([{ appkitSession: 1, answer: 'ab'.repeat(16), token: TOKEN }]);
    stop();
    expect(old.listening()).toBe(0);
  });
});
