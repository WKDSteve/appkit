import { describe, expect, it } from 'vitest';
import { MAX_ITEMS, WriteQueue, type QueueStore, type SendOutcome } from './queue';

type Op = { op: 'new'; title: string } | { op: 'set'; id: string; status: string; if_match?: string };

function validOp(v: unknown): v is Op {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as Record<string, unknown>;
  if (o['op'] === 'new') return typeof o['title'] === 'string';
  if (o['op'] === 'set') return typeof o['id'] === 'string' && typeof o['status'] === 'string';
  return false;
}

function memory(): QueueStore & { saved: unknown[] | null } {
  const s = {
    saved: null as unknown[] | null,
    async load() {
      return s.saved;
    },
    async save(items: unknown[]) {
      s.saved = JSON.parse(JSON.stringify(items)) as unknown[];
    },
    async clear() {
      s.saved = null;
    },
  };
  return s;
}

function queue(store = memory()) {
  let n = 0;
  return {
    store,
    q: new WriteQueue<Op>({
      store,
      validOp,
      needsIfMatch: (op) => op.op === 'set',
      ifMatchOf: (op) => (op.op === 'set' ? op.if_match : undefined),
      newId: () => `op-${String(++n).padStart(8, '0')}`,
      now: () => new Date('2026-11-11T10:00:00Z'),
    }),
  };
}

describe('WriteQueue', () => {
  it('shows a conflict with the new version and overwrites nothing', async () => {
    const { q } = queue();
    await q.add({ op: 'set', id: 'a/1', status: 'done', if_match: 'v1' }, 'a', 'Task 1');
    const sent: Op[] = [];
    const report = await q.flush(async (_id, op) => {
      sent.push(op);
      return { kind: 'conflict', message: 'the file changed', current: { id: 'a/1', status: 'blocked', version: 'v2' } };
    });
    expect(report).toEqual({ sent: 0, stopped: null });
    expect(q.items).toHaveLength(1);
    expect(q.items[0]).toMatchObject({
      state: 'conflict',
      message: 'the file changed',
      current: { status: 'blocked', version: 'v2' },
    });
    // Never sent again, not by a second run and not by a retry.
    await q.retry(q.items[0]!.clientOpId);
    await q.flush(async (_id, op) => {
      sent.push(op);
      return { kind: 'done' };
    });
    expect(sent).toHaveLength(1);
    expect(q.items[0]!.state).toBe('conflict');
    // The person drops it.
    await q.drop(q.items[0]!.clientOpId);
    expect(q.items).toHaveLength(0);
  });

  it('keeps a change only with the version the person saw', async () => {
    const { q } = queue();
    await expect(q.add({ op: 'set', id: 'a/1', status: 'done' }, 'a', 'x')).rejects.toThrow('if_match');
    await q.add({ op: 'new', title: 'n' }, 'a', 'n');
    expect(q.items).toHaveLength(1);
  });

  it('never sends a write again whose answer was lost, and stops a run that cannot reach the hub', async () => {
    const { q } = queue();
    await q.add({ op: 'new', title: 'one' }, 'a', 'one');
    await q.add({ op: 'new', title: 'two' }, 'a', 'two');
    await q.add({ op: 'new', title: 'three' }, 'a', 'three');
    const answers: SendOutcome[] = [{ kind: 'lost' }, { kind: 'unreachable' }];
    const sent: string[] = [];
    const report = await q.flush(async (_id, op) => {
      sent.push(op.op === 'new' ? op.title : '');
      return answers.shift()!;
    });
    expect(report).toEqual({ sent: 0, stopped: 'unreachable' });
    expect(sent).toEqual(['one', 'two']);
    expect(q.items.map((i) => i.state)).toEqual(['unknown', 'waiting', 'waiting']);
    // A thrown error of the transport is a lost answer too.
    const r2 = await q.flush(async (_id, op) => {
      if (op.op === 'new' && op.title === 'two') throw new Error('reset');
      return { kind: 'done' };
    });
    expect(r2.sent).toBe(1);
    expect(q.items.map((i) => [i.label, i.state])).toEqual([
      ['one', 'unknown'],
      ['two', 'unknown'],
    ]);
    // Only the person sends it again.
    await q.retry(q.items[0]!.clientOpId);
    expect(q.items[0]!.state).toBe('waiting');
  });

  it('reads back only what it wrote, and a write cut off by the end of the app is unknown', async () => {
    const store = memory();
    const { q } = queue(store);
    await q.add({ op: 'new', title: 'kept' }, 'a', 'kept');
    await q.add({ op: 'set', id: 'b/1', status: 'done', if_match: 'v' }, 'b', 'other area');
    (store.saved![0] as Record<string, unknown>)['state'] = 'sending';
    store.saved!.push(
      { clientOpId: 'short', op: { op: 'new', title: 'x' }, scope: 'a', label: 'x', queuedAt: '', state: 'waiting' },
      { clientOpId: 'op-99999999', op: { op: 'rm' }, scope: 'a', label: 'x', queuedAt: '', state: 'waiting' },
      { clientOpId: 'op-99999998', op: { op: 'set', id: 'a/2', status: 'x' }, scope: 'a', label: 'x', queuedAt: '', state: 'waiting' },
      { clientOpId: 'op-99999997', op: { op: 'new', title: 'x' }, scope: 'a', label: 'x', queuedAt: '', state: 'done' },
      'garbage',
    );
    const { q: again } = queue(store);
    await again.load(new Set(['a']));
    expect(again.items.map((i) => [i.label, i.state])).toEqual([['kept', 'unknown']]);
    await again.clear();
    expect(store.saved).toBeNull();
  });

  it('holds at most its limit', async () => {
    const { q } = queue();
    for (let i = 0; i < MAX_ITEMS; i++) await q.add({ op: 'new', title: `${i}` }, 'a', `${i}`);
    expect(q.full).toBe(true);
    await expect(q.add({ op: 'new', title: 'more' }, 'a', 'more')).rejects.toThrow('full');
  });
});
