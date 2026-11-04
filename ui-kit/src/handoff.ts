// The session for a new tab. The token is kept per tab (session storage): a tab that the browser opened from a link
// (the context menu's "Open in new tab", a middle click it handled itself) starts without it, and without a token the
// page would ask for a new pairing code. Such a tab asks the open tabs of the page for theirs, over a broadcast
// channel; one that has a session answers with its token. The new tab then shares that session: no new one is made
// (the hub holds few), and the pairing code stays a thing for the first tab.
//
// A broadcast channel reaches only pages of the same origin (scheme, host and port) in the same browser profile, and
// a script of that origin owns the session anyway. A tab answers only while its event stream is open: the hub is then
// up on this very port, so the page that asks is the page that hub served, and not a page of another program that took
// the port after the hub stopped. What this means for the session is in SECURITY.md.

/** The part of a `BroadcastChannel` this needs. */
export interface ChannelLike {
  postMessage(message: unknown): void;
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void;
  removeEventListener(type: 'message', listener: (event: { data: unknown }) => void): void;
}

/**
 * How long a new tab waits for an answer before it offers to pair the way a first tab does. On a busy machine an open
 * tab can take seconds to answer (it is in the background, its process is the last to run): see `onLate`.
 */
export const ASK_MS = 400;

/** A token as the hub hands it out: printable ASCII, not long. Anything else in an answer is not taken. */
const TOKEN = /^[\x21-\x7e]{16,512}$/;
const NONCE = /^[0-9a-f]{32}$/;

interface Ask {
  appkitSession: 1;
  ask: string;
}
interface Answer {
  appkitSession: 1;
  answer: string;
  token: string;
}

function askOf(data: unknown): string | null {
  if (typeof data !== 'object' || data === null) return null;
  const m = data as Partial<Ask>;
  return m.appkitSession === 1 && typeof m.ask === 'string' && NONCE.test(m.ask) ? m.ask : null;
}

function answerOf(data: unknown, nonce: string): string | null {
  if (typeof data !== 'object' || data === null) return null;
  const m = data as Partial<Answer>;
  if (m.appkitSession !== 1 || m.answer !== nonce || typeof m.token !== 'string') return null;
  return TOKEN.test(m.token) ? m.token : null;
}

export interface ShareOptions {
  channel: ChannelLike;
  /** The token of this tab, or null without a session. */
  token: () => string | null;
  /** The event stream of this tab is open (`StreamSession.open`): only then does it answer. */
  live: () => boolean;
}

/** Answer the new tabs of the page with this tab's session. Returns the function that stops it. */
export function shareSession(options: ShareOptions): () => void {
  const onMessage = (event: { data: unknown }): void => {
    const nonce = askOf(event.data);
    if (nonce === null || !options.live()) return;
    const token = options.token();
    if (token === null || !TOKEN.test(token)) return;
    options.channel.postMessage({ appkitSession: 1, answer: nonce, token } satisfies Answer);
  };
  options.channel.addEventListener('message', onMessage);
  return () => options.channel.removeEventListener('message', onMessage);
}

export interface AskOptions {
  channel: ChannelLike;
  /** 16 random bytes (`crypto.getRandomValues`): the answer is to this question, not to another tab's. */
  random: () => Uint8Array;
  setTimeout: (run: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
  /** Default `ASK_MS`. */
  waitMs?: number;
  /**
   * An answer after the wait (the promise has already said null): the page may still take the token. Called at most
   * once. Without it the question ends with the wait. With it the listener stays until an answer comes, which costs one
   * function on a channel the page keeps anyway.
   */
  onLate?: (token: string) => void;
}

/** Ask the open tabs of the page for a session: the token of the first that answers, or null after the wait. */
export function askForSession(options: AskOptions): Promise<string | null> {
  const bytes = options.random();
  const nonce = [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
  if (!NONCE.test(nonce)) return Promise.resolve(null);
  return new Promise((resolve) => {
    let timer: unknown = null;
    let waited = false;
    const stopListening = (): void => options.channel.removeEventListener('message', onMessage);
    const onMessage = (event: { data: unknown }): void => {
      const token = answerOf(event.data, nonce);
      if (token === null) return;
      stopListening();
      if (waited) {
        options.onLate?.(token);
        return;
      }
      if (timer !== null) options.clearTimeout(timer);
      resolve(token);
    };
    options.channel.addEventListener('message', onMessage);
    timer = options.setTimeout(() => {
      waited = true;
      if (options.onLate === undefined) stopListening();
      resolve(null);
    }, options.waitMs ?? ASK_MS);
    options.channel.postMessage({ appkitSession: 1, ask: nonce } satisfies Ask);
  });
}
