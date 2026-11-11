import { describe, expect, it, vi } from 'vitest';
import { Preferences, StreamSession, type StreamDeps, type StreamNews, type StreamShare } from './session.svelte';
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

  it('reads on at once with a new token when a follow comes during the wait, even if the wait never ends', async () => {
    const tokens: (string | null)[] = [];
    const { session, d, log } = deps([], {
      fetchFn: (async (_url: string, init: RequestInit) => {
        const bearer = new Headers(init.headers).get('Authorization');
        tokens.push(bearer);
        if (bearer === 'Bearer old') throw new TypeError('the stream is down');
        return new Response('', { status: 401 });
      }) as typeof fetch,
      bearer: () => 'old',
      sleep: () => new Promise<void>(() => {}),
    });
    const first = session.follow(d);
    await vi.waitFor(() => expect(session.connection).toBe('reconnecting'));

    const second = session.follow({ ...d, bearer: () => 'new' });
    await Promise.all([first, second]);
    expect(tokens).toEqual(['Bearer old', 'Bearer new']);
    expect(log).toEqual(['lost']);
  });

  it('a wait that was cut short does not cut the next one short when it ends', async () => {
    const tokens: (string | null)[] = [];
    const ends: (() => void)[] = [];
    const { session, d } = deps([], {
      fetchFn: (async (_url: string, init: RequestInit) => {
        tokens.push(new Headers(init.headers).get('Authorization'));
        throw new TypeError('the stream is down');
      }) as typeof fetch,
      sleep: () => new Promise<void>((resolve) => void ends.push(resolve)),
    });
    const reader = session.follow(d);
    await vi.waitFor(() => expect(ends).toHaveLength(1));
    void session.follow(d);
    await vi.waitFor(() => expect(ends).toHaveLength(2));
    expect(tokens).toHaveLength(2);

    ends[0]!();
    await new Promise((resolve) => setTimeout(resolve));
    expect(tokens).toHaveLength(2);

    session.stop();
    ends[1]!();
    await reader;
  });

  it('a wait that fails still ends the reader with its error', async () => {
    const { session, d } = deps([], {
      fetchFn: (async () => {
        throw new TypeError('the stream is down');
      }) as typeof fetch,
      sleep: () => Promise.reject(new Error('no timer')),
    });
    await expect(session.follow(d)).rejects.toThrow('no timer');
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

/** The lock and the channel of one browser, for windows that are sessions of this test. */
function browser() {
  const heard = new Set<(news: StreamNews) => void>();
  const queue: { granted: () => Promise<void>; done: (v: unknown) => void; fail: (e: unknown) => void }[] = [];
  let held = false;
  const next = () => {
    if (held) return;
    const first = queue.shift();
    if (!first) return;
    held = true;
    void first.granted().then(first.done, first.fail).finally(() => {
      held = false;
      next();
    });
  };
  const share: StreamShare = {
    name: 'one-session',
    locks: {
      request: (_name, options, granted) =>
        new Promise((done, fail) => {
          const entry = { granted, done, fail };
          queue.push(entry);
          options.signal?.addEventListener('abort', () => {
            const at = queue.indexOf(entry);
            if (at >= 0) {
              queue.splice(at, 1);
              fail(new DOMException('given up', 'AbortError'));
            }
          });
          next();
        }),
    },
    open(listener) {
      heard.add(listener);
      return {
        post: (news) => {
          for (const other of [...heard]) if (other !== listener) other(news);
        },
        close: () => void heard.delete(listener),
      };
    },
  };
  return share;
}

/** A hub: its streams stay open, say `tasks` once, and are counted while they last. */
function hubOfStreams() {
  const hub = { open: 0, most: 0, status: 200 };
  const fetchFn = (async (_url: unknown, init?: RequestInit) => {
    if (hub.status !== 200) return new Response('', { status: hub.status });
    hub.open++;
    hub.most = Math.max(hub.most, hub.open);
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('event: tasks\ndata: {}\n\n'));
        init?.signal?.addEventListener('abort', () => {
          hub.open--;
          controller.error(new DOMException('cut', 'AbortError'));
        });
      },
    });
    return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
  }) as typeof fetch;
  return { hub, fetchFn };
}

function aWindow(share: StreamShare, fetchFn: typeof fetch) {
  // A real wait: a hub that stays down must not run the guard of `deps` out while the test looks.
  const sleep = () => new Promise<void>((resolve) => setTimeout(resolve, 5));
  const made = deps([], { fetchFn, sleep, share: () => share });
  const reading = made.session.follow(made.d);
  return { ...made, reading };
}

describe('StreamSession, shared by the windows of a session', () => {
  it('holds one stream for eight windows, and all of them are live and hear it', async () => {
    const share = browser();
    const { hub, fetchFn } = hubOfStreams();
    const first = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(first.log).toContain('message:tasks'));
    const others = Array.from({ length: 7 }, () => aWindow(share, fetchFn));
    for (const w of others) {
      await vi.waitFor(() => expect(w.session.open).toBe(true));
      expect(w.session.connection).toBe('live');
      expect(w.log).toEqual(['open']);
    }
    expect(hub.most).toBe(1);
    for (const w of [first, ...others]) w.session.stop();
    await Promise.all([first, ...others].map((w) => w.reading));
    expect(hub.open).toBe(0);
  });

  it('the next window takes over when the one that held it ends, and every window reads again', async () => {
    const share = browser();
    const { hub, fetchFn } = hubOfStreams();
    const first = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(first.session.open).toBe(true));
    const second = aWindow(share, fetchFn);
    const third = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(third.session.open).toBe(true));
    await vi.waitFor(() => expect(second.session.open).toBe(true));
    const [opensOfSecond, opensOfThird] = [second, third].map((w) => w.log.filter((l) => l === 'open').length);

    first.session.stop();
    await first.reading;
    await vi.waitFor(() => expect(second.log.filter((l) => l === 'open').length).toBe((opensOfSecond ?? 0) + 1));
    await vi.waitFor(() => expect(third.log.filter((l) => l === 'open').length).toBe((opensOfThird ?? 0) + 1));
    // What the new holder reads, the other one hears.
    expect(third.log).toContain('message:tasks');
    expect(hub.open).toBe(1);
    expect(hub.most).toBe(1);
    second.session.stop();
    third.session.stop();
    await Promise.all([second.reading, third.reading]);
  });

  it('a refused session reaches every window, and none reads on', async () => {
    const share = browser();
    const { hub, fetchFn } = hubOfStreams();
    const first = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(first.session.open).toBe(true));
    const second = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(second.session.open).toBe(true));
    hub.status = 401;
    first.session.cut();
    await Promise.all([first.reading, second.reading]);
    expect(first.log).toContain('lost');
    expect(second.log).toContain('lost');
  });

  it('a stream that is down shows in the window that only hears it', async () => {
    const share = browser();
    const { hub, fetchFn } = hubOfStreams();
    const first = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(first.session.open).toBe(true));
    const second = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(second.session.open).toBe(true));
    hub.status = 503;
    first.session.cut();
    await vi.waitFor(() => expect(second.session.connection).toBe('reconnecting'));
    expect(second.session.open).toBe(false);
    first.session.stop();
    second.session.stop();
    await Promise.all([first.reading, second.reading]);
  });

  it('a browser that gives no lock leaves each window its own stream', async () => {
    const share: StreamShare = {
      ...browser(),
      locks: { request: () => Promise.reject(new DOMException('no', 'SecurityError')) },
    };
    const { hub, fetchFn } = hubOfStreams();
    const w = aWindow(share, fetchFn);
    await vi.waitFor(() => expect(w.session.open).toBe(true));
    expect(hub.open).toBe(1);
    w.session.stop();
    await w.reading;
  });

  it('a new token is a new session: its window leaves the old share', async () => {
    const share = browser();
    const other: StreamShare = { ...browser(), name: 'another-session' };
    const { hub, fetchFn } = hubOfStreams();
    const made = deps([], { fetchFn, share: () => share });
    const reading = made.session.follow(made.d);
    await vi.waitFor(() => expect(made.session.open).toBe(true));
    let asked = 0;
    made.session.follow({
      ...made.d,
      share: () => {
        asked++;
        return other;
      },
    });
    await vi.waitFor(() => expect(asked).toBeGreaterThan(0));
    await vi.waitFor(() => expect(made.session.open).toBe(true));
    expect(hub.open).toBe(1);
    made.session.stop();
    await reading;
  });

  it('a window that takes over answers down until its own stream opens', async () => {
    const share = browser();
    const { hub, fetchFn } = hubOfStreams();
    // After the first stream, the hub takes the next request and never answers it.
    let streams = 0;
    const slow = ((url: string, init?: RequestInit) =>
      streams++ === 0 ? fetchFn(url, init) : new Promise<Response>(() => {})) as typeof fetch;
    const first = aWindow(share, slow);
    await vi.waitFor(() => expect(first.session.open).toBe(true));
    const second = aWindow(share, slow);
    await vi.waitFor(() => expect(second.session.open).toBe(true));

    first.session.stop();
    await first.reading;
    await vi.waitFor(() => expect(streams).toBe(2));
    const answers: string[] = [];
    const probe = share.open((news) => void answers.push(news.kind));
    probe.post({ kind: 'ask' });
    await vi.waitFor(() => expect(answers).toEqual(['down']));
    expect(second.session.open).toBe(false);
    probe.close();
    second.session.stop();
    expect(hub.open).toBe(0);
  });
});

describe('StreamSession.stop', () => {
  it('ends the wait before the next try at once', async () => {
    const { session, d, sleeps } = deps([() => new Response('', { status: 503 })], {
      // A wait that only a stop can end.
      sleep: (ms) => {
        sleeps.push(ms);
        return new Promise<void>(() => {});
      },
    });
    const reading = session.follow(d);
    await vi.waitFor(() => expect(sleeps).toEqual([1000]));
    session.stop();
    const late = new Promise((resolve) => setTimeout(() => resolve('still waiting'), 200));
    await expect(Promise.race([reading.then(() => 'ended'), late])).resolves.toBe('ended');
  });
});
