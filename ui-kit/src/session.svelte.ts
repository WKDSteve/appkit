// The part of the state of a page that every app of appkit has the same way: what the person chose for the whole page
// (the language, the theme, whether single keys are shortcuts), and the connection to the hub (the event stream that is
// read again after a break, backing off, until the session is gone). What a page does with a message of the stream, and
// with the loss of the session, is the app's (`StreamDeps`); how it starts, trades a pairing code and shows data is too.

import { pickLang } from './i18n';
import { backoffSeconds, readStream, StreamRefused, type SseMessage } from './sse';

// ── what the person chose for the page ────────────────────────────────────────────────────────────────────────────

export type Theme = 'auto' | 'light' | 'dark';

/** Per-browser storage; every failure of it is a miss. */
export interface Profile {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** The keys under which the choices are kept. */
export interface PreferenceKeys {
  lang: string;
  theme: string;
  shortcuts: string;
}

function safe<T>(work: () => T): T | null {
  try {
    return work();
  } catch {
    return null;
  }
}

export class Preferences<L extends string> {
  lang = $state<L>() as L;
  theme = $state<Theme>('auto');
  /** Single keys (letters, `/`) are shortcuts; off for those whose assistive software uses them. */
  shortcuts = $state(true);

  /**
   * Read the choices back; what is not a choice the page makes is the default (the language of the browser, the
   * system theme, shortcuts on).
   * @param langs the languages the app has
   * @param preferred the languages the browser asks for, best first
   * @param fallback the language when none of them is one of the app
   */
  constructor(
    private readonly profile: Profile,
    private readonly keys: PreferenceKeys,
    private readonly langs: readonly L[],
    preferred: readonly string[],
    fallback: L,
  ) {
    const lang = safe(() => profile.getItem(keys.lang));
    this.lang = (langs as readonly string[]).includes(lang ?? '') ? (lang as L) : pickLang(preferred, langs, fallback);
    const theme = safe(() => profile.getItem(keys.theme));
    this.theme = theme === 'light' || theme === 'dark' ? theme : 'auto';
    this.shortcuts = safe(() => profile.getItem(keys.shortcuts)) !== 'off';
  }

  setLang(lang: L): void {
    this.lang = lang;
    safe(() => this.profile.setItem(this.keys.lang, lang));
  }

  setTheme(theme: Theme): void {
    this.theme = theme;
    safe(() => this.profile.setItem(this.keys.theme, theme));
  }

  setShortcuts(on: boolean): void {
    this.shortcuts = on;
    safe(() => this.profile.setItem(this.keys.shortcuts, on ? 'on' : 'off'));
  }
}

// ── the connection to the hub ─────────────────────────────────────────────────────────────────────────────────────

export type Connection = 'live' | 'reconnecting' | 'offline';

/** What the stream needs of the page and the hub. */
export interface StreamDeps {
  fetchFn: typeof fetch;
  /** The address of the stream, and the token to read it with. */
  url(): string;
  bearer(): string;
  /** Whether there is a session: without one the stream is not read. */
  hasToken(): boolean;
  sleep(ms: number): Promise<void>;
  /** The stream is open (what happened while it was down is to be read again). */
  onOpen(): void;
  onMessage(message: SseMessage): void;
  /** The hub refused the session (a 401): the page is no longer paired. */
  onLost(): void;
  /**
   * How the windows of one session share one stream, or null when they cannot: then this window reads its own. Asked
   * again at each start, because a new token is a new session.
   */
  share?: () => StreamShare | null;
}

/** What the window that holds the stream tells the others of its session. */
export type StreamNews =
  /** A window that just came asks where the stream stands. */
  | { kind: 'ask' }
  /** The stream opened: what happened while it was down is to be read again, in every window. */
  | { kind: 'opened' }
  /** The answer to `ask`: it is open. A window that knew that has nothing to do. */
  | { kind: 'live' }
  /** The stream is down, and being read again. */
  | { kind: 'down' }
  | { kind: 'message'; message: SseMessage }
  /** The hub refused the session. */
  | { kind: 'lost' };

/** The two things of a browser that windows share a stream with. A test has its own. */
export interface StreamShare {
  /** The same for every window of a session, and for no other session. */
  name: string;
  locks: {
    request(name: string, options: { signal?: AbortSignal }, granted: () => Promise<void>): Promise<unknown>;
  };
  /** Hear the other windows; what is posted reaches them, not this one. */
  open(heard: (news: StreamNews) => void): { post(news: StreamNews): void; close(): void };
}

/** A short name for a token: it names the session, and need not hide anything from the same origin. */
function scopeOfToken(token: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < token.length; i++) {
    hash ^= token.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

/**
 * The share of this browser for the session of `token`, or null where there are no Web Locks or no BroadcastChannel
 * (locks need a secure context, which a plain address other than localhost is not).
 */
export function browserStreamShare(token: string): StreamShare | null {
  const locks = globalThis.navigator?.locks;
  if (!locks || typeof globalThis.BroadcastChannel !== 'function') return null;
  const name = `appkit-stream:${scopeOfToken(token)}`;
  return {
    name,
    locks,
    open(heard) {
      const channel = new BroadcastChannel(name);
      channel.onmessage = (event: MessageEvent<StreamNews>) => heard(event.data);
      return { post: (news) => channel.postMessage(news), close: () => channel.close() };
    },
  };
}

export class StreamSession {
  connection = $state<Connection>('offline');
  /** The stream is open now. */
  open = $state(false);
  /** The page was stopped: nothing is read, nothing started again. */
  stopped = false;
  private abort: AbortController | null = null;
  private deps: StreamDeps | null = null;
  private reading = false;
  private reader: Promise<void> = Promise.resolve();
  /** Ends the wait before the next try, while there is one. */
  private endWait: (() => void) | null = null;
  /** Stops waiting for the stream of the session, while this window hears it from another. */
  private leave: (() => void) | null = null;
  /** Tells the other windows, while this one holds the stream. */
  private publish: ((news: StreamNews) => void) | null = null;
  /** The browser turned the lock down: this window reads its own stream from now on. */
  private cannotShare = false;
  /** What the hub refused: a `follow` since then brought something else, which is read. */
  private refused: StreamDeps | null = null;

  /** The page starts (or starts again after it was stopped). */
  resume(): void {
    this.stopped = false;
  }

  /**
   * The page stops: the stream that is open is cut, a wait for the next try ends, and the loop ends (and with it the
   * lock this window held, so the next window takes over at once rather than after the wait).
   */
  stop(): void {
    this.stopped = true;
    this.abort?.abort();
    this.endWait?.();
    this.leave?.();
  }

  /** Cut the stream that is open (the session is gone); the loop ends when it sees there is no session. */
  cut(): void {
    this.abort?.abort();
    this.leave?.();
  }

  /**
   * Read the stream, and again after it breaks, a little later each time up to half a minute, until the page stops, the
   * session is gone or the hub refuses it.
   *
   * There is one reader. A page that stops and starts again at once (a pairing link pasted into the tab) finds the one
   * that ran still going: it is not joined by a second, its stream is cut, a wait for the next try ends, and it reads on
   * with what this call brings.
   *
   * With `deps.share`, one window of a session holds the stream and the others hear it from that one; when it ends,
   * the next one takes over and every window reads again what happened in between.
   */
  follow(deps: StreamDeps): Promise<void> {
    this.deps = deps;
    if (this.reading) {
      this.abort?.abort();
      this.endWait?.();
      this.leave?.();
      return this.reader;
    }
    this.reading = true;
    this.reader = this.read();
    return this.reader;
  }

  private async read(): Promise<void> {
    this.refused = null;
    try {
      for (;;) {
        const deps = this.deps;
        if (deps === null || this.stopped || !deps.hasToken()) return;
        const share = this.shareOf(deps);
        await (share === null ? this.hold(null) : this.shared(share));
        if (this.refused === this.deps) return;
      }
    } finally {
      // In the same step as the last check of the loop: a later `follow` never finds a reader that has already left.
      this.reading = false;
    }
  }

  private shareOf(deps: StreamDeps): StreamShare | null {
    return this.cannotShare ? null : (deps.share?.() ?? null);
  }

  /** Hear the stream from the window that holds it, until the lock comes to this one, and then hold it. */
  private async shared(share: StreamShare): Promise<void> {
    const leaving = new AbortController();
    let holding = false;
    const opened = () => {
      this.open = true;
      this.connection = 'live';
      this.deps?.onOpen();
    };
    const channel = share.open((news) => {
      if (holding) {
        if (news.kind === 'ask') channel.post({ kind: this.open ? 'live' : 'down' });
        return;
      }
      if (news.kind === 'opened') opened();
      else if (news.kind === 'live' && !this.open) opened();
      else if (news.kind === 'down') {
        this.open = false;
        this.connection = 'reconnecting';
      } else if (news.kind === 'message') this.deps?.onMessage(news.message);
      else if (news.kind === 'lost') {
        this.refused = this.deps;
        this.deps?.onLost();
        leaving.abort();
      }
    });
    this.leave = () => leaving.abort();
    channel.post({ kind: 'ask' });
    try {
      await share.locks.request(share.name, { signal: leaving.signal }, async () => {
        holding = true;
        this.leave = null;
        // What this window heard as a listener is not its own stream: until that opens, it answers `down`.
        this.open = false;
        this.publish = (news) => channel.post(news);
        await this.hold(share.name);
      });
    } catch (error) {
      // A wait that was given up is no failure; a lock the browser will not give means no sharing here. What went
      // wrong while this window held the stream is the reader's own.
      if (holding) throw error;
      if (!leaving.signal.aborted) this.cannotShare = true;
    } finally {
      this.publish = null;
      this.leave = null;
      this.open = false;
      channel.close();
    }
  }

  /**
   * Read the stream, again and again, until the session is gone, the page stopped or the session is another one than
   * `scope`.
   */
  private async hold(scope: string | null): Promise<void> {
    let attempt = 0;
    for (;;) {
      const deps = this.deps;
      if (deps === null || this.stopped || !deps.hasToken()) return;
      if ((this.shareOf(deps)?.name ?? null) !== scope) return;
      this.abort = new AbortController();
      try {
        await readStream(
          deps.fetchFn,
          deps.url(),
          deps.bearer(),
          {
            onOpen: () => {
              attempt = 0;
              this.open = true;
              this.connection = 'live';
              this.publish?.({ kind: 'opened' });
              deps.onOpen();
            },
            onMessage: (m) => {
              this.publish?.({ kind: 'message', message: m });
              deps.onMessage(m);
            },
          },
          this.abort.signal,
        );
      } catch (error) {
        if (error instanceof StreamRefused && error.status === 401) {
          this.publish?.({ kind: 'lost' });
          this.refused = deps;
          deps.onLost();
          return;
        }
      } finally {
        this.open = false;
      }
      const now = this.deps;
      if (now === null || this.stopped || !now.hasToken()) return;
      this.connection = 'reconnecting';
      this.publish?.({ kind: 'down' });
      await this.wait(deps, backoffSeconds(attempt++) * 1000);
    }
  }

  /** Sleep before the next try, which a `follow` cuts short. */
  private async wait(deps: StreamDeps, ms: number): Promise<void> {
    try {
      await new Promise<void>((resolve, reject) => {
        this.endWait = resolve;
        deps.sleep(ms).then(resolve, reject);
      });
    } finally {
      this.endWait = null;
    }
  }
}
