import { describe, expect, it } from 'vitest';
import { installNav, placeOf, type NavAddress, type NavHistory, type NavWindow } from './nav';

interface S {
  view: string;
  id: string | null;
}
const isS = (v: unknown): v is S =>
  typeof v === 'object' && v !== null && typeof (v as S).view === 'string' && ((v as S).id === null || typeof (v as S).id === 'string');

function fixture() {
  const calls: Array<[string, unknown, unknown]> = [];
  let listener: ((e: { state: unknown }) => void) | null = null;
  const history: NavHistory = {
    pushState: (d, _u, url) => void calls.push(['push', d, url]),
    replaceState: (d, _u, url) => void calls.push(['replace', d, url]),
  };
  const win: NavWindow = {
    addEventListener: (_t, l) => void (listener = l),
    removeEventListener: () => void (listener = null),
  };
  const restored: S[] = [];
  const nav = installNav<S>({ history, window: win, isState: isS, restore: (s) => void restored.push(s) });
  return { calls, restored, nav, pop: (state: unknown) => listener?.({ state }), listening: () => listener !== null };
}

describe('installNav', () => {
  it('marks the first place as the entry the page started on and makes an entry for each new place', () => {
    const f = fixture();
    f.nav.record({ view: 'overview', id: null });
    f.nav.record({ view: 'rules', id: null });
    f.nav.record({ view: 'rules', id: 'AA-01' });
    expect(f.calls.map((c) => c[0])).toEqual(['replace', 'push', 'push']);
  });

  it('does not add an entry for the same place twice', () => {
    const f = fixture();
    f.nav.record({ view: 'rules', id: null });
    f.nav.record({ view: 'rules', id: null });
    expect(f.calls).toHaveLength(1);
  });

  it('never gives a URL: the address, and the fragment of a pairing link, stay as they are', () => {
    const f = fixture();
    f.nav.record({ view: 'rules', id: null });
    f.nav.record({ view: 'families', id: null });
    expect(f.calls.every((c) => c[2] === undefined)).toBe(true);
  });

  it('restores the state of the entry Back brings, and that is not a new entry', () => {
    const f = fixture();
    f.nav.record({ view: 'overview', id: null });
    f.nav.record({ view: 'rules', id: 'AA-01' });
    const before = f.calls.length;
    f.pop(f.calls[0]?.[1]);
    expect(f.restored).toEqual([{ view: 'overview', id: null }]);
    // The page reports the restored state back: it is the current place, so no entry is made.
    f.nav.record({ view: 'overview', id: null });
    expect(f.calls).toHaveLength(before);
    // Going on from there is a new entry again.
    f.nav.record({ view: 'families', id: null });
    expect(f.calls.at(-1)?.[0]).toBe('push');
  });

  it('takes the place of the current entry when told to, and only then', () => {
    const f = fixture();
    f.nav.record({ view: 'overview', id: null });
    f.nav.record({ view: 'rules', id: 'AA-01' });
    // The open item is gone: the place is the list again, in the entry the item had.
    f.nav.record({ view: 'rules', id: null }, { replace: true });
    expect(f.calls.map((c) => c[0])).toEqual(['replace', 'push', 'replace']);
    expect(f.calls[2]?.[1]).toEqual({ appkitNav: 1, state: { view: 'rules', id: null } });
    // Without it a changed place is an entry again.
    f.nav.record({ view: 'families', id: null });
    expect(f.calls.at(-1)?.[0]).toBe('push');
    // The same place twice is nothing, replace or not.
    f.nav.record({ view: 'families', id: null }, { replace: true });
    expect(f.calls).toHaveLength(4);
  });

  it('leaves alone an entry it did not make or that holds something else', () => {
    const f = fixture();
    f.nav.record({ view: 'overview', id: null });
    for (const foreign of [null, undefined, 'x', {}, { appkitNav: 2, state: { view: 'a', id: null } }, { appkitNav: 1, state: { view: 3 } }]) {
      f.pop(foreign);
    }
    expect(f.restored).toEqual([]);
  });

  it('stops listening', () => {
    const f = fixture();
    expect(f.listening()).toBe(true);
    f.nav.stop();
    expect(f.listening()).toBe(false);
  });
});

describe('installNav with the place in the address', () => {
  const VIEWS = ['overview', 'rules', 'families'];
  function addressed(hash = '') {
    const calls: Array<[string, unknown, unknown]> = [];
    let listener: ((e: { state: unknown }) => void) | null = null;
    const location = { pathname: '/', search: '', hash };
    const setUrl = (url: unknown) => {
      if (typeof url === 'string') location.hash = url.includes('#') ? url.slice(url.indexOf('#')) : '';
    };
    const history: NavHistory = {
      pushState: (d, _u, url) => {
        calls.push(['push', d, url]);
        setUrl(url);
      },
      replaceState: (d, _u, url) => {
        calls.push(['replace', d, url]);
        setUrl(url);
      },
    };
    const win: NavWindow = {
      addEventListener: (_t, l) => void (listener = l),
      removeEventListener: () => void (listener = null),
    };
    const restored: S[] = [];
    const address: NavAddress<S> = {
      location,
      toRoute: (s): Record<string, string> => (s.id === null ? { view: s.view } : { view: s.view, item: s.id }),
      fromRoute: (r) => (r.view !== undefined && VIEWS.includes(r.view) ? { view: r.view, id: r.item ?? null } : null),
    };
    const nav = installNav<S>({ history, window: win, isState: isS, restore: (s) => void restored.push(s), address });
    return { calls, restored, nav, location, address, pop: (state: unknown) => listener?.({ state }) };
  }

  it('puts each place into the fragment, the first one too', () => {
    const f = addressed();
    f.nav.record({ view: 'overview', id: null });
    f.nav.record({ view: 'rules', id: 'AA 01' });
    expect(f.calls.map((c) => [c[0], c[2]])).toEqual([
      ['replace', '/#view=overview'],
      ['push', '/#view=rules&item=AA%2001'],
    ]);
  });

  it('a reload or a link opens the place in the address; a pairing link and a fragment without a place do not', () => {
    expect(placeOf(addressed('#view=rules&item=AA-01').address)).toEqual({ view: 'rules', id: 'AA-01' });
    expect(placeOf(addressed('#view=nothing').address)).toBeNull();
    expect(placeOf(addressed('').address)).toBeNull();
    expect(placeOf(addressed(`#pair=${'ab'.repeat(16)}&view=rules`).address)).toBeNull();
  });

  it('leaves the address to the pairing while a code is in it', () => {
    const f = addressed(`#pair=${'ab'.repeat(16)}`);
    f.nav.record({ view: 'overview', id: null });
    expect(f.calls).toEqual([['replace', { appkitNav: 1, state: { view: 'overview', id: null } }, undefined]]);
  });

  it('goes to a place typed into the address, and makes its entry one of ours', () => {
    const f = addressed();
    f.nav.record({ view: 'overview', id: null });
    f.location.hash = '#view=families';
    f.pop(null);
    expect(f.restored).toEqual([{ view: 'families', id: null }]);
    expect(f.calls.at(-1)).toEqual(['replace', { appkitNav: 1, state: { view: 'families', id: null } }, '/#view=families']);
    // The page reports it back: no entry.
    const before = f.calls.length;
    f.nav.record({ view: 'families', id: null });
    expect(f.calls).toHaveLength(before);
  });

  it('a pasted pairing link or a fragment without a place is not a place', () => {
    const f = addressed();
    f.nav.record({ view: 'overview', id: null });
    for (const hash of [`#pair=${'ab'.repeat(16)}&view=rules`, '#view=nothing', '']) {
      f.location.hash = hash;
      f.pop(null);
    }
    expect(f.restored).toEqual([]);
    expect(f.calls).toHaveLength(1);
  });

  it('Back still restores from the entry, not from the address', () => {
    const f = addressed();
    f.nav.record({ view: 'overview', id: null });
    f.nav.record({ view: 'rules', id: null });
    f.location.hash = '#view=families';
    f.pop(f.calls[0]?.[1]);
    expect(f.restored).toEqual([{ view: 'overview', id: null }]);
  });
});
