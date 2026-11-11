// The page side of the mobile basis (rules `MOB-06`, `MUI-05`, `MOB-09` of the ruleset `mobile`). The page of a phone
// app never speaks to the hub itself: the shell around it (`appkit-mobile` in a Tauri app) holds the way, the device key
// and the session, and the page asks it for a route of its app over the bridge. `ipcFetch` turns that bridge into a
// `fetch`, so the app's own client (`HubClient` and what extends it) works unchanged; the page sees the way only as a word
// to show (`Way`) and the abilities it leaves out what it cannot do with.

/** How the phone reaches the data: Tailscale Serve, an SSH tunnel, or a vault cloned on the phone. */
export type Way = 'tailnet' | 'tunnel' | 'vault';

/** What a way can do (the same table as `appkit_mobile::way::Way::abilities`). The hub still decides per device. */
export interface WayAbilities {
  read: boolean;
  /** Create and change, through the hub, with `if_match`. */
  write: boolean;
  /** Start something on the PC, with a confirmation on the phone. */
  trigger: boolean;
  /** Notices of the hub (Web Push) can reach the phone through this way. */
  push: boolean;
}

export const WAY_ABILITIES: Readonly<Record<Way, WayAbilities>> = {
  tailnet: { read: true, write: true, trigger: true, push: true },
  tunnel: { read: true, write: true, trigger: true, push: false },
  vault: { read: true, write: false, trigger: false, push: false },
};

export function isWay(value: unknown): value is Way {
  return value === 'tailnet' || value === 'tunnel' || value === 'vault';
}

/**
 * Why the shell had no answer for the page, as one word (`appkit_mobile::link::LinkError`): the page shows this state
 * and the next step, never an empty list (`MOB-09`).
 *
 * - `unreachable`: nothing reached the hub (Tailscale off, tunnel down, PC asleep); trying again is safe.
 * - `answer_lost`: it may have arrived; a write is not sent again by itself (`MOB-07`).
 * - `too_large`: the answer was over the limit.
 * - `unpaired`: the hub does not take this phone any more (revoked): pair again.
 * - `other_hub`: another program answers where the hub was: nothing was signed; pair again.
 * - `not_allowed`: the route is not the app's: nothing was sent (a fault of the page).
 * - `cancelled`: the person did not confirm.
 * - `failed`: anything else.
 */
export type LinkFailure =
  | 'unreachable'
  | 'answer_lost'
  | 'too_large'
  | 'unpaired'
  | 'other_hub'
  | 'not_allowed'
  | 'cancelled'
  | 'failed';

const FAILURES: readonly LinkFailure[] = [
  'unreachable',
  'answer_lost',
  'too_large',
  'unpaired',
  'other_hub',
  'not_allowed',
  'cancelled',
  'failed',
];

/** A request of the page that the shell could not answer. */
export class LinkError extends Error {
  constructor(readonly kind: LinkFailure) {
    super(kind);
  }
}

/** What the page sends over the bridge: a route of its app (path and query), never a whole address. */
export interface BridgeRequest {
  method: 'GET' | 'POST';
  path: string;
  body: string | null;
  ifNoneMatch: string | null;
}

/** What the shell answers. */
export interface BridgeResponse {
  status: number;
  contentType: string | null;
  etag: string | null;
  body: string;
  /**
   * The hub was not reached and this is the copy the shell kept (`MOB-08`): when the hub sent it, seconds since 1970.
   * The `Response` carries it as `X-Kept-At`, so the page can say how old what it shows is.
   */
  keptAt?: number | null;
}

/** The header of a `Response` of `ipcFetch` that says the answer is the kept copy, and from when (seconds). */
export const KEPT_AT = 'X-Kept-At';

/** The bridge to the shell: Tauri's `invoke` of the command the app names, or a stand-in in tests. */
export type Bridge = (request: BridgeRequest) => Promise<BridgeResponse>;

function failureOf(error: unknown): LinkFailure {
  const kind = typeof error === 'string' ? error : (error as { kind?: unknown } | null)?.kind;
  return FAILURES.includes(kind as LinkFailure) ? (kind as LinkFailure) : 'failed';
}

function isResponse(value: unknown): value is BridgeResponse {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    Number.isInteger(v['status']) &&
    (v['status'] as number) >= 100 &&
    (v['status'] as number) <= 599 &&
    typeof v['body'] === 'string' &&
    (v['contentType'] === null || typeof v['contentType'] === 'string') &&
    (v['etag'] === null || typeof v['etag'] === 'string') &&
    (v['keptAt'] === undefined || v['keptAt'] === null || (Number.isInteger(v['keptAt']) && (v['keptAt'] as number) >= 0))
  );
}

/**
 * A `fetch` that goes over the bridge to the shell. Only a path of this page's own origin is taken (`/api/...`): an
 * absolute address is refused here, before anything crosses the bridge, and the shell refuses a route that is not the
 * app's again. No header but `If-None-Match` and the JSON type is passed on: the session is the shell's, and an
 * `Authorization` the page might set is dropped.
 */
export function ipcFetch(bridge: Bridge): typeof fetch {
  return async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (!raw.startsWith('/') || raw.startsWith('//')) throw new LinkError('not_allowed');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method !== 'GET' && method !== 'POST') throw new LinkError('not_allowed');
    const headers = new Headers(init?.headers);
    const body = init?.body;
    if (body !== undefined && body !== null && typeof body !== 'string') throw new LinkError('not_allowed');
    let answer: unknown;
    try {
      answer = await bridge({
        method,
        path: raw,
        body: method === 'POST' ? (body ?? '{}') : null,
        ifNoneMatch: headers.get('If-None-Match'),
      });
    } catch (error) {
      throw new LinkError(failureOf(error));
    }
    if (!isResponse(answer)) throw new LinkError('failed');
    const out = new Headers();
    if (answer.contentType) out.set('Content-Type', answer.contentType);
    if (answer.etag) out.set('ETag', answer.etag);
    if (typeof answer.keptAt === 'number') out.set(KEPT_AT, String(answer.keptAt));
    // A 304 has no body; the constructor refuses one.
    return new Response(answer.status === 304 ? null : answer.body, { status: answer.status, headers: out });
  };
}

/** What the page shows about the connection, always (`MUI-05`): the way, whether it answered, and how old the data is. */
export interface Connection {
  way: Way;
  /** `null`: not asked yet. */
  reachable: boolean | null;
  /** When the data on screen came from the hub (or the vault), ms since 1970; `null`: never. */
  fetchedAt: number | null;
  /** Why the last request failed, if it did. */
  failure: LinkFailure | null;
}

/** The next step for a failure, as a key the app's dictionary words (`mobile.next.<key>`). */
export function nextStep(failure: LinkFailure, way: Way): string {
  switch (failure) {
    case 'unreachable':
      return way === 'tailnet' ? 'tailscale' : way === 'tunnel' ? 'tunnel' : 'vault';
    case 'unpaired':
    case 'other_hub':
      return 'pair';
    case 'answer_lost':
      return 'look';
    case 'cancelled':
      return 'confirm';
    default:
      return 'retry';
  }
}
