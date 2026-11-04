import { describe, expect, it, vi } from 'vitest';
import { Preferences, StreamSession, type StreamDeps } from './session.svelte';
import type { SseMessage } from './sse';

function profile(initial: Record<string, string> = {}, broken = false) {
  const store = new Map(Object.entries(initial));
  return {
    store,
    getItem: (k: string) => {
      if (broken) throw new Error('blocked');
      return store.get(k) ?? null;
    },
    setItem: (k: string, v: string) => {
      if (broken) throw new Error('blocked');
      store.set(k, v);
    },
  };
}
const KEYS = { lang: 'a.lang', theme: 'a.theme', shortcuts: 'a.shortcuts' };

describe('Preferences', () => {
  it('reads the choices back', () => {
    const p = profile({ 'a.lang': 'de', 'a.theme': 'dark', 'a.shortcuts': 'off' });
    const prefs = new Preferences(p, KEYS, ['de', 'en'], ['en'], 'en');
    expect([prefs.lang, prefs.theme, prefs.shortcuts]).toEqual(['de', 'dark', false]);
  });

  it('takes the language of the browser, the system theme and shortcuts on when nothing is chosen', () => {
    const prefs = new Preferences(profile(), KEYS, ['de', 'en'], ['de-AT', 'en'], 'en');
    expect([prefs.lang, prefs.theme, prefs.shortcuts]).toEqual(['de', 'auto', true]);
    expect(new Preferences(profile(), KEYS, ['de', 'en'], ['fr'], 'en').lang).toBe('en');
  });

  it('takes a damaged profile for the defaults, and a storage that is blocked too', () => {
    const damaged = profile({ 'a.lang': '<img src=x>', 'a.theme': 'purple', 'a.shortcuts': 'maybe' });
    const prefs = new Preferences(damaged, KEYS, ['de', 'en'], [], 'en');
    expect([prefs.lang, prefs.theme, prefs.shortcuts]).toEqual(['en', 'auto', true]);
    const blocked = new Preferences(profile({}, true), KEYS, ['de', 'en'], [], 'de');
    expect(blocked.lang).toBe('de');
    // A choice that cannot be kept is still made for this page.
    blocked.setLang('en');
    blocked.setTheme('light');
    blocked.setShortcuts(false);
    expect([blocked.lang, blocked.theme, blocked.shortcuts]).toEqual(['en', 'light', false]);
  });

  it('keeps each choice in the profile under its own key', () => {
    const p = profile();
    const prefs = new Preferences(p, KEYS, ['de', 'en'], [], 'en');
    prefs.setLang('de');
    prefs.setTheme('dark');
    prefs.setShortcuts(false);
    expect([...p.store]).toEqual([
      ['a.lang', 'de'],
      ['a.theme', 'dark'],
      ['a.shortcuts', 'off'],
    ]);
    prefs.setShortcuts(true);
    expect(p.store.get('a.shortcuts')).toBe('on');
  });
});

/** An event stream that gives these messages and ends. */
function streamOf(...messages: string[]): Response {
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(messages.map((m) => `event: ${m}\ndata: {}\n\n`).join('')));
      controller.close();
    },
  });
  return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
}

function deps(answers: (() => Response)[], over: Partial<StreamDeps> = {}) {
  const log: string[] = [];
  const sleeps: number[] = [];
  const session = new StreamSession();
  const d: StreamDeps = {
    fetchFn: (async () => {
      log.push('fetch');
      const next = answers.shift();
      if (!next) throw new Error('no more');
      return next();
    }) as typeof fetch,
    url: () => '/events',
    bearer: () => 'tok',
    hasToken: () => true,
    sleep: async (ms) => {
      sleeps.push(ms);
      // A loop that does not end fails the test instead of hanging it.
      if (sleeps.length > 50) throw new Error('the loop does not end');
    },
    onOpen: () => void log.push('open'),
    onMessage: (m: SseMessage) => void log.push(`message:${m.event}`),
    onLost: () => void log.push('lost'),
    ...over,
  };
  return { session, d, log, sleeps };
}

describe('StreamSession', () => {
  it('opens, hands over the messages, and reads again after the stream ends, a little later each time', async () => {
    const sleeps: number[] = [];
    const { session, d, log } = deps([() => streamOf('tasks'), () => streamOf('resync'), () => streamOf('tasks')], {
      sleep: async (ms) => {
        sleeps.push(ms);
        if (sleeps.length === 3) session.stop();
      },
    });
    await session.follow(d);
    expect(log.filter((l) => l !== 'fetch')).toEqual(['open', 'message:tasks', 'open', 'message:resync', 'open', 'message:tasks']);
    // The pause grows only while no stream opened: each one opened, so it starts from the first again.
    expect(sleeps).toEqual([1000, 1000, 1000]);
    expect(session.connection).toBe('reconnecting');
    expect(session.open).toBe(false);
  });

  it('is live while the stream is open', async () => {
    const seen: string[] = [];
    const { session, d } = deps([() => streamOf('hello')], {
      onOpen: () => void seen.push(session.connection),
      sleep: async () => session.stop(),
    });
    await session.follow(d);
    expect(seen).toEqual(['live']);
  });

  it('backs off when the stream does not open, up to half a minute', async () => {
    const sleeps: number[] = [];
    const refusals = Array.from({ length: 8 }, () => () => new Response('', { status: 503 }));
    const { session, d } = deps(refusals, {
      sleep: async (ms) => {
        sleeps.push(ms);
        if (sleeps.length === 8) session.stop();
      },
    });
    await session.follow(d);
    expect(sleeps).toEqual([1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]);
  });

  it('says the session is lost on a 401, reads no more, and does not wait', async () => {
    const { session, d, log, sleeps } = deps([() => new Response('', { status: 401 })]);
    await session.follow(d);
    expect(log).toEqual(['fetch', 'lost']);
    expect(sleeps).toEqual([]);
  });

  it('reads nothing without a session, and ends when the session goes while it waits', async () => {
    const none = deps([() => streamOf('tasks')], { hasToken: () => false });
    await none.session.follow(none.d);
    expect(none.log).toEqual([]);
    let held = true;
    const gone = deps([() => streamOf('tasks')], { hasToken: () => held, sleep: async () => void (held = false) });
    await gone.session.follow(gone.d);
    expect(gone.log.filter((l) => l === 'fetch')).toHaveLength(1);
  });

  it('does not say the session is lost when the page was stopped while the stream broke', async () => {
    const { session, d, log } = deps([() => new Response('', { status: 401 })]);
    session.stop();
    await session.follow(d);
    expect(log).toEqual([]);
    session.resume();
    expect(session.stopped).toBe(false);
  });

  /** A hub whose streams stay open until they are cut, and that says how many are open and with which token. */
  function holdingHub() {
    const tokens: string[] = [];
    let open = 0;
    const fetchFn = (async (_url: string, init: RequestInit) => {
      tokens.push(new Headers(init.headers).get('Authorization') ?? '');
      open += 1;
      const body = new ReadableStream<Uint8Array>({
        start(controller) {
          init.signal?.addEventListener('abort', () => {
            open -= 1;
            controller.error(new DOMException('cut', 'AbortError'));
          });
        },
      });
      return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
    }) as unknown as typeof fetch;
    return { fetchFn, tokens, openStreams: () => open };
  }

  it('stays one reader when a page stops and starts again at once, and reads on with the new token', async () => {
    const hub = holdingHub();
    const { session, d } = deps([], { fetchFn: hub.fetchFn, bearer: () => 'old' });
    const first = session.follow(d);
    await vi.waitFor(() => expect(hub.openStreams()).toBe(1));

    // What a page does with a pairing link pasted into the tab.
    session.stop();
    session.resume();
    const second = session.follow({ ...d, bearer: () => 'new' });
    await vi.waitFor(() => expect(hub.tokens.at(-1)).toBe('Bearer new'));
    await new Promise((resolve) => setTimeout(resolve));
    expect(hub.openStreams()).toBe(1);
    expect(hub.tokens).toEqual(['Bearer old', 'Bearer new']);

    session.stop();
    await Promise.all([first, second]);
    expect(hub.openStreams()).toBe(0);
  });

  it('starts a new reader once the last one has ended', async () => {
    const { session, d, log } = deps([() => new Response('', { status: 401 }), () => new Response('', { status: 401 })]);
    await session.follow(d);
    await session.follow(d);
    expect(log).toEqual(['fetch', 'lost', 'fetch', 'lost']);
  });

  it('reads after a follow that had no session and ended at once', async () => {
    const none = deps([() => new Response('', { status: 401 })], { hasToken: () => false });
    await none.session.follow(none.d);
    await none.session.follow({ ...none.d, hasToken: () => true });
    expect(none.log).toEqual(['fetch', 'lost']);
  });

  it('does not join a reader at the very moment it ends', async () => {
    let second: Promise<void> = Promise.resolve();
    const { session, d, log } = deps([() => new Response('', { status: 401 })]);
    const later = { ...d, hasToken: () => true };
    const ending: StreamDeps = {
      ...d,
      hasToken: () => {
        // Runs after the check that ends the reader, before anything that waits for its end.
        queueMicrotask(() => {
          second = session.follow(later);
        });
        return false;
      },
    };
    await session.follow(ending);
    await second;
    expect(log).toEqual(['fetch', 'lost']);
  });
});
