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

  /** The page starts (or starts again after it was stopped). */
  resume(): void {
    this.stopped = false;
  }

  /** The page stops: the stream that is open is cut, and the loop ends. */
  stop(): void {
    this.stopped = true;
    this.abort?.abort();
  }

  /** Cut the stream that is open (the session is gone); the loop ends when it sees there is no session. */
  cut(): void {
    this.abort?.abort();
  }

  /**
   * Read the stream, and again after it breaks, a little later each time up to half a minute, until the page stops, the
   * session is gone or the hub refuses it.
   *
   * There is one reader. A page that stops and starts again at once (a pairing link pasted into the tab) finds the one
   * that ran still going: it is not joined by a second, its stream is cut, a wait for the next try ends, and it reads on
   * with what this call brings.
   */
  follow(deps: StreamDeps): Promise<void> {
    this.deps = deps;
    if (this.reading) {
      this.abort?.abort();
      this.endWait?.();
      return this.reader;
    }
    this.reading = true;
    this.reader = this.read();
    return this.reader;
  }

  private async read(): Promise<void> {
    try {
      let attempt = 0;
      for (;;) {
        const deps = this.deps;
        if (deps === null || this.stopped || !deps.hasToken()) return;
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
                deps.onOpen();
              },
              onMessage: (m) => deps.onMessage(m),
            },
            this.abort.signal,
          );
        } catch (error) {
          if (error instanceof StreamRefused && error.status === 401) {
            deps.onLost();
            return;
          }
        } finally {
          this.open = false;
        }
        const now = this.deps;
        if (now === null || this.stopped || !now.hasToken()) return;
        this.connection = 'reconnecting';
        await this.wait(deps, backoffSeconds(attempt++) * 1000);
      }
    } finally {
      // In the same step as the last check of the loop: a later `follow` never finds a reader that has already ended.
      this.reading = false;
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
