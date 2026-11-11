// The writes of a phone, kept until the hub said yes or no (rules `MOB-07`, `MOB-08`, `MUI-04` of the ruleset
// `mobile`). Taken from tasks-mobile, where it was written, without what only tasks has (files of a new task): an app
// names its operations and how to send one, the queue does the rest.
//
// - Nothing is shown as done before the hub said so: the page shows the data as the hub sent it, and the queue beside
//   it as "waiting".
// - A change carries `if_match`, the version the person saw. When the hub says the data moved on (a conflict), the item
//   keeps the hub's words and the current version, if the hub sent one, and is never sent again: the newer version is
//   not overwritten. The person looks at it and drops the item.
// - A write whose answer was lost is `unknown` and is never sent again by itself (a second "new" would make a second
//   entry); the person looks and decides. One that was being sent when the app was ended is `unknown` after the start.
// - Starting something (a run, a scene) is never queued: it needs the person and the PC at that moment.

export type PendingState = 'waiting' | 'sending' | 'unknown' | 'conflict' | 'refused';

export interface Pending<O> {
  /** The id of the write, sent with it: a hub that knows it was done tells a repeat. */
  clientOpId: string;
  op: O;
  /** Where the write goes (an area): an item of a scope that may no longer be shown is dropped when read back. */
  scope: string;
  /** What the list shows for it. */
  label: string;
  queuedAt: string;
  state: PendingState;
  /** The hub's words, for `conflict` and `refused`. */
  message?: string;
  code?: string;
  /** The version the hub has now, for a conflict: shown beside the item, never written over. */
  current?: unknown;
}

/** What sending one write gave. */
export type SendOutcome =
  | { kind: 'done' }
  | { kind: 'conflict'; message: string; current?: unknown }
  | { kind: 'refused'; code: string; message: string }
  /** It did not reach the hub: stays waiting, the run stops. */
  | { kind: 'unreachable' }
  /** The hub does not take this phone any more: stays waiting, the run stops. */
  | { kind: 'unpaired' }
  /** The hub is busy (429, 503): stays waiting, the run stops. */
  | { kind: 'busy' }
  /** It may have arrived: `unknown`. */
  | { kind: 'lost' };

/** Where the queue is kept (storage of the phone; memory in tests). */
export interface QueueStore {
  load(): Promise<unknown>;
  save(items: unknown[]): Promise<void>;
  clear(): Promise<void>;
}

export interface QueueDeps<O> {
  store: QueueStore;
  /** Is this what the app queues? Read back from storage, every item is checked (`MSEC-13`). */
  validOp: (op: unknown) => op is O;
  /** A change (not a new entry) must carry `if_match`: it is queued only with the version the person saw. */
  needsIfMatch: (op: O) => boolean;
  ifMatchOf: (op: O) => string | undefined;
  newId: () => string;
  now: () => Date;
  /** Called after every change of `items` (the page redraws). */
  changed?: () => void;
}

export interface FlushReport {
  sent: number;
  /** Why the run stopped before the end, if it did. */
  stopped: 'unreachable' | 'unpaired' | 'busy' | null;
}

export const MAX_ITEMS = 200;
const STATES: readonly PendingState[] = ['waiting', 'sending', 'unknown', 'conflict', 'refused'];

export class WriteQueue<O> {
  items: Pending<O>[] = [];
  private flushing = false;

  constructor(private readonly deps: QueueDeps<O>) {}

  /** What was read back from storage, checked; items of a scope not in `shown` (when given) are dropped. */
  restore(raw: unknown, shown: ReadonlySet<string> | null): Pending<O>[] {
    if (!Array.isArray(raw)) return [];
    const items: Pending<O>[] = [];
    for (const entry of raw.slice(0, MAX_ITEMS)) {
      if (typeof entry !== 'object' || entry === null) continue;
      const e = entry as Record<string, unknown>;
      if (typeof e['clientOpId'] !== 'string' || !/^[A-Za-z0-9-]{8,64}$/.test(e['clientOpId'])) continue;
      const op = e['op'];
      if (!this.deps.validOp(op)) continue;
      if (this.deps.needsIfMatch(op) && typeof this.deps.ifMatchOf(op) !== 'string') continue;
      if (typeof e['scope'] !== 'string' || typeof e['label'] !== 'string' || typeof e['queuedAt'] !== 'string') continue;
      if (!(STATES as readonly unknown[]).includes(e['state'])) continue;
      if (shown !== null && !shown.has(e['scope'])) continue;
      const state = e['state'] as PendingState;
      items.push({
        clientOpId: e['clientOpId'],
        op,
        scope: e['scope'],
        label: e['label'].slice(0, 500),
        queuedAt: e['queuedAt'],
        // Ended while it was being sent: it may have arrived.
        state: state === 'sending' ? 'unknown' : state,
        ...(typeof e['message'] === 'string' ? { message: e['message'].slice(0, 500) } : {}),
        ...(typeof e['code'] === 'string' ? { code: e['code'].slice(0, 48) } : {}),
        ...(e['current'] !== undefined ? { current: e['current'] } : {}),
      });
    }
    return items;
  }

  async load(shown: ReadonlySet<string> | null): Promise<void> {
    let raw: unknown = null;
    try {
      raw = await this.deps.store.load();
    } catch {
      raw = null;
    }
    this.items = this.restore(raw, shown);
    this.deps.changed?.();
  }

  private async persist(): Promise<void> {
    this.deps.changed?.();
    await this.deps.store.save(this.items.map((item) => ({ ...item })));
  }

  get full(): boolean {
    return this.items.length >= MAX_ITEMS;
  }

  /**
   * Put a write into the queue. A change without `if_match` is refused here: offline, a change is only kept with the
   * version the person saw.
   */
  async add(op: O, scope: string, label: string): Promise<Pending<O>> {
    if (this.full) throw new Error('queue full');
    if (this.deps.needsIfMatch(op) && typeof this.deps.ifMatchOf(op) !== 'string') throw new Error('if_match required');
    const item: Pending<O> = {
      clientOpId: this.deps.newId(),
      op,
      scope,
      label,
      queuedAt: this.deps.now().toISOString(),
      state: 'waiting',
    };
    this.items.push(item);
    await this.persist();
    return item;
  }

  /** Send what waits, one after the other, in the order it was queued. One run at a time. */
  async flush(send: (clientOpId: string, op: O) => Promise<SendOutcome>): Promise<FlushReport> {
    if (this.flushing) return { sent: 0, stopped: null };
    this.flushing = true;
    let sent = 0;
    const tried = new Set<Pending<O>>();
    try {
      for (;;) {
        const item = this.items.find((i) => i.state === 'waiting' && !tried.has(i));
        if (!item) break;
        tried.add(item);
        item.state = 'sending';
        await this.persist();
        let outcome: SendOutcome;
        try {
          outcome = await send(item.clientOpId, item.op);
        } catch {
          outcome = { kind: 'lost' };
        }
        switch (outcome.kind) {
          case 'done': {
            const at = this.items.indexOf(item);
            if (at >= 0) this.items.splice(at, 1);
            sent += 1;
            break;
          }
          case 'conflict':
            item.state = 'conflict';
            item.code = 'conflict';
            item.message = outcome.message.slice(0, 500);
            if (outcome.current !== undefined) item.current = outcome.current;
            break;
          case 'refused':
            item.state = 'refused';
            item.code = outcome.code.slice(0, 48);
            item.message = outcome.message.slice(0, 500);
            break;
          case 'lost':
            item.state = 'unknown';
            break;
          case 'unreachable':
          case 'unpaired':
          case 'busy':
            item.state = 'waiting';
            await this.persist();
            return { sent, stopped: outcome.kind };
        }
        await this.persist();
      }
      return { sent, stopped: null };
    } finally {
      this.flushing = false;
    }
  }

  /**
   * The person looked and wants an `unknown` or refused write sent again (same id). A conflict is not sent again: it
   * carries the old version and would be refused again; the person drops it and acts on the new version.
   */
  async retry(clientOpId: string): Promise<void> {
    const item = this.items.find((i) => i.clientOpId === clientOpId);
    if (!item || (item.state !== 'unknown' && item.state !== 'refused')) return;
    item.state = 'waiting';
    delete item.message;
    delete item.code;
    await this.persist();
  }

  /** The person drops a write (it arrived after all, or it is not wanted any more). Not while it is being sent. */
  async drop(clientOpId: string): Promise<void> {
    const before = this.items.length;
    this.items = this.items.filter((i) => i.clientOpId !== clientOpId || i.state === 'sending');
    if (this.items.length !== before) await this.persist();
  }

  /** Forget everything (the phone is unpaired: `MSEC-12`). */
  async clear(): Promise<void> {
    this.items = [];
    this.deps.changed?.();
    await this.deps.store.clear();
  }
}
