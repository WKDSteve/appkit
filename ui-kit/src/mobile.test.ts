import { describe, expect, it } from 'vitest';
import { ipcFetch, LinkError, nextStep, WAY_ABILITIES, type BridgeRequest, type BridgeResponse } from './mobile';
import { HubClient } from './client';

function bridge(answer: BridgeResponse | (() => never)) {
  const seen: BridgeRequest[] = [];
  const fn = async (r: BridgeRequest) => {
    seen.push(r);
    if (typeof answer === 'function') return answer();
    return answer;
  };
  return { seen, fn };
}

const OK: BridgeResponse = { status: 200, contentType: 'application/json', etag: '"abc"', body: '{"a":1}' };

describe('ipcFetch', () => {
  it('passes a route of the page and gives a Response back, and an app client works over it unchanged', async () => {
    const b = bridge(OK);
    const client = new HubClient(ipcFetch(b.fn), '/api/testing');
    client.setToken('page-token-is-dropped');
    const doc = await client.json<{ a: number }>('/api/testing/fleet');
    expect(doc).toEqual({ a: 1 });
    expect(b.seen).toEqual([{ method: 'GET', path: '/api/testing/fleet', body: null, ifNoneMatch: null }]);
  });

  it('sends a POST with its JSON and a GET with If-None-Match; a 304 has no body', async () => {
    const b = bridge({ status: 304, contentType: null, etag: '"t"', body: '' });
    const f = ipcFetch(b.fn);
    const r = await f('/api/testing/fleet', { headers: { 'If-None-Match': '"t"', Authorization: 'Bearer x' } });
    expect(r.status).toBe(304);
    expect(r.headers.get('etag')).toBe('"t"');
    await f('/api/testing/rerun', { method: 'POST', body: '{"key":"a"}' });
    expect(b.seen[0]).toEqual({ method: 'GET', path: '/api/testing/fleet', body: null, ifNoneMatch: '"t"' });
    expect(b.seen[1]).toEqual({ method: 'POST', path: '/api/testing/rerun', body: '{"key":"a"}', ifNoneMatch: null });
  });

  it('refuses an address of elsewhere, another method or a body that is not text before anything crosses', async () => {
    const b = bridge(OK);
    const f = ipcFetch(b.fn);
    for (const url of ['https://evil.example/api/x', '//evil.example/api/x', 'api/testing/fleet']) {
      await expect(f(url)).rejects.toEqual(new LinkError('not_allowed'));
    }
    await expect(f('/api/testing/x', { method: 'DELETE' })).rejects.toBeInstanceOf(LinkError);
    await expect(f('/api/testing/x', { method: 'POST', body: new Blob(['x']) })).rejects.toBeInstanceOf(LinkError);
    expect(b.seen).toHaveLength(0);
  });

  it('turns a refusal of the shell into its word, and anything else into failed', async () => {
    for (const [thrown, kind] of [
      [{ kind: 'unreachable' }, 'unreachable'],
      ['unpaired', 'unpaired'],
      [{ kind: 'other_hub' }, 'other_hub'],
      [{ kind: '<script>' }, 'failed'],
      [new Error('x'), 'failed'],
    ] as const) {
      const f = ipcFetch(
        bridge(() => {
          throw thrown;
        }).fn,
      );
      await expect(f('/api/testing/fleet')).rejects.toEqual(new LinkError(kind));
    }
    const bad = ipcFetch(async () => ({ status: 99, body: '', contentType: null, etag: null }));
    await expect(bad('/api/testing/fleet')).rejects.toEqual(new LinkError('failed'));
  });
});

describe('ways', () => {
  it('a cloned vault only reads; push comes through the tailnet only', () => {
    expect(WAY_ABILITIES.vault).toEqual({ read: true, write: false, trigger: false, push: false });
    expect(WAY_ABILITIES.tailnet.push).toBe(true);
    expect(WAY_ABILITIES.tunnel.push).toBe(false);
  });

  it('names the next step for each failure', () => {
    expect(nextStep('unreachable', 'tailnet')).toBe('tailscale');
    expect(nextStep('unreachable', 'tunnel')).toBe('tunnel');
    expect(nextStep('unpaired', 'tailnet')).toBe('pair');
    expect(nextStep('other_hub', 'tunnel')).toBe('pair');
    expect(nextStep('answer_lost', 'tailnet')).toBe('look');
  });
});
