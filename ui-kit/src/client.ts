// The part of the hub client that every app shares: the session as a bearer token and nothing else (no cookie, no
// cache, no referrer), the refusal of the hub as an error, the pairing code traded for a token, and the calls every hub
// has (hello, snapshot with its tag, rescan). An app extends `HubClient` with the calls of its own data
// (`class Hub extends HubClient`) and names where its routes live (`/api/tasks`).

/** The refusal of the hub as an error: its status, its code and its message. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** The refusal as an error: the code and message of the hub, or the status when the body is not the hub's. */
export async function failure(response: Response): Promise<ApiError> {
  let code = 'http_error';
  let message = `HTTP ${response.status}`;
  try {
    const doc = (await response.json()) as { error?: { code?: unknown; message?: unknown } };
    if (typeof doc.error?.code === 'string') code = doc.error.code;
    if (typeof doc.error?.message === 'string') message = doc.error.message;
  } catch {
    // not JSON: keep the generic words
  }
  return new ApiError(response.status, code, message);
}

/** What a request of the page is: no cache, no cookie, no referrer. */
export const PLAIN: Pick<RequestInit, 'cache' | 'credentials' | 'referrerPolicy'> = {
  cache: 'no-store',
  credentials: 'omit',
  referrerPolicy: 'no-referrer',
};

/** `hello` of the core: the hub and whether its engine is ready. Readable without a session. */
export interface Hello {
  instance: string;
  version: string;
  /** The short commit id of a rolling build; `null` or missing for a release (older hubs omit it). */
  build?: string | null;
  engine: { ready: boolean; problem: string | null; capabilities: string[] | null };
}

/** The snapshot of an app, or the word that the one held is still the current one (304). */
export type SnapshotResult<S> = { kind: 'fresh'; doc: S; etag: string | null } | { kind: 'not_modified' };

/** The tag of a response without the marks of a weak tag and the quotes. */
export function etagOf(raw: string | null): string | null {
  return raw ? raw.replace(/^W\//, '').replace(/^"|"$/g, '') : null;
}

/** A text of a document of the hub, or `null` when it is not one: the page takes only what it shows. */
export function textOf(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

/** @typeParam S the snapshot of the app (`Snapshot` of its contract). */
export class HubClient<S = unknown> {
  private token: string | null = null;
  /** Called when the hub says the session is gone. */
  onUnauthorized: () => void = () => {};

  /**
   * @param fetchFn the `fetch` of the page (replaced in the tests)
   * @param ns where the routes of the app live, `/api/tasks`
   * @param base the origin, empty for the page's own
   */
  constructor(
    protected readonly fetchFn: typeof fetch,
    readonly ns: string,
    protected readonly base: string = '',
  ) {}

  setToken(token: string | null): void {
    this.token = token;
  }

  get hasToken(): boolean {
    return this.token !== null;
  }

  /** The session token, for a request the browser makes by itself (the event stream). */
  get bearer(): string {
    return this.token ?? '';
  }

  url(path: string): string {
    return this.base + path;
  }

  /** The path of a route of the app: `route('snapshot')` is `/api/tasks/snapshot`. */
  route(name: string): string {
    return `${this.ns}/${name}`;
  }

  /** The address of the event stream. */
  get eventsUrl(): string {
    return this.url(this.route('events'));
  }

  async request(path: string, init: RequestInit = {}): Promise<Response> {
    const headers = new Headers(init.headers);
    const sent = this.token;
    if (sent) headers.set('Authorization', `Bearer ${sent}`);
    const response = await this.fetchFn(this.url(path), { ...init, headers, ...PLAIN });
    // The session that was refused is the one that is gone: a refusal that comes after another token was set (a late
    // answer to a question asked with an old one) says nothing about the new one.
    if (response.status === 401 && this.token === sent) this.onUnauthorized();
    return response;
  }

  async json<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await this.request(path, init);
    if (!response.ok) throw await failure(response);
    return (await response.json()) as T;
  }

  /** A POST with a JSON body, the answer read as JSON. */
  postJson<T>(path: string, body: unknown, signal?: AbortSignal): Promise<T> {
    return this.json<T>(path, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      ...(signal ? { signal } : {}),
    });
  }

  /** A POST with a JSON body whose answer is only "it went through". */
  async post(path: string, body: unknown): Promise<void> {
    const response = await this.request(path, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!response.ok) throw await failure(response);
  }

  async hello(): Promise<Hello> {
    const doc = await this.json<{
      instance: string;
      version: string;
      engine: { ready: boolean; problem: string | null; capabilities?: string[] | null };
    }>(this.route('hello'));
    return {
      instance: doc.instance,
      version: doc.version,
      engine: { ready: doc.engine.ready, problem: doc.engine.problem, capabilities: doc.engine.capabilities ?? null },
    };
  }

  /** Trade a pairing code for a session token. */
  async exchange(code: string): Promise<string> {
    const response = await this.fetchFn(this.url('/auth/exchange'), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code }),
      ...PLAIN,
    });
    if (!response.ok) throw await failure(response);
    const doc = (await response.json()) as { token?: unknown };
    if (typeof doc.token !== 'string') throw new ApiError(502, 'bad_reply', 'no token');
    return doc.token;
  }

  /** The snapshot; with the tag of the one held, a 304 says it is still the current one. */
  async snapshot(etag: string | null): Promise<SnapshotResult<S>> {
    const headers: Record<string, string> = {};
    if (etag) headers['If-None-Match'] = `"${etag}"`;
    const response = await this.request(this.route('snapshot'), { headers });
    if (response.status === 304) return { kind: 'not_modified' };
    if (!response.ok) throw await failure(response);
    return { kind: 'fresh', doc: (await response.json()) as S, etag: etagOf(response.headers.get('etag')) };
  }

  /** Ask the hub to look at the files again (the window got the focus back). */
  rescan(): Promise<void> {
    return this.post(this.route('rescan'), {});
  }
}
