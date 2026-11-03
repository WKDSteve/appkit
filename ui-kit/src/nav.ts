// Back and forward in the browser: each place the person reaches (a view, an open item) is an entry of the
// session history of the tab, and Back and Forward bring the app to the state that entry holds.
//
// The state lives in `history.state`. An app that gives `address` also puts each place into the fragment of the
// address (`route.ts`): a reload then opens the same place, the address can be copied, and a link to a place opens it
// in a new tab. Without `address` the URL is left as it is (`pushState` without one). An address that carries a
// pairing code is the pairing's (`pair.ts`): it is neither read as a place nor written over.
//
// The kit knows no view and no item: the app says what its state is (`record`), checks what comes back from the
// history (`isState`) and puts the page into a state (`restore`).

import { hasPairing, readRoute, routeHash, type Route } from './route';

export interface NavHistory {
  pushState(data: unknown, unused: string, url?: string | URL | null): void;
  replaceState(data: unknown, unused: string, url?: string | URL | null): void;
}

export interface NavWindow {
  addEventListener(type: 'popstate', listener: (event: { state: unknown }) => void): void;
  removeEventListener(type: 'popstate', listener: (event: { state: unknown }) => void): void;
}

/** The place in the address: the app's route of a state, and the state of a route. */
export interface NavAddress<S> {
  location: { pathname: string; search: string; hash: string };
  /**
   * The route of the state. Only what a new tab needs to open the place; the rest stays in `history.state`. Null: the
   * address is not this state's to set (another part of the page keeps it, a window of one item), it is left as it is.
   */
  toRoute: (state: S) => Route | null;
  /** The state a route names, or null when it names none (no route, a view that is no view). */
  fromRoute: (route: Record<string, string>) => S | null;
}

export interface NavOptions<S> {
  history: NavHistory;
  window: NavWindow;
  /** Is this what the app put into the history? An entry that is not (another page's, an old one) is left alone. */
  isState: (value: unknown) => value is S;
  /** Bring the page to this state. It must not call `record` for it: the entry exists already. */
  restore: (state: S) => void;
  /** Are two states the same place? Default: the same JSON. */
  same?: (a: S, b: S) => boolean;
  /** Keep the place in the address too (see the head of this file). */
  address?: NavAddress<S>;
}

export interface Nav<S> {
  /**
   * The page is at this state now. The first call marks the entry the page started on; a changed state is a new
   * entry, unless `replace` is set: then it takes the place of the current entry. That is for a place that the page
   * left by itself (the item that was open is gone): an entry of its own for it would be one that Back leads to and
   * the page leaves again at once, which holds Back at that entry for ever. It is also for a change that is not a
   * step of its own (a filter): the place changes, Back does not walk through it.
   */
  record(state: S, options?: { replace?: boolean }): void;
  /** Stop listening. */
  stop(): void;
}

/** What is stored: marked, so that an entry of something else is not taken for ours. */
interface Entry<S> {
  appkitNav: 1;
  state: S;
}

function entryOf<S>(value: unknown, isState: (v: unknown) => v is S): Entry<S> | null {
  if (typeof value !== 'object' || value === null) return null;
  const entry = value as { appkitNav?: unknown; state?: unknown };
  return entry.appkitNav === 1 && isState(entry.state) ? { appkitNav: 1, state: entry.state } : null;
}

/**
 * The state the address names, or null (no address kept, a pairing link, no place in it). What a page that starts
 * opens first: a link to a place, or the place a reload left.
 */
export function placeOf<S>(address: NavAddress<S>): S | null {
  const hash = address.location.hash;
  if (hasPairing(hash)) return null;
  return address.fromRoute(readRoute(hash));
}

export function installNav<S>(options: NavOptions<S>): Nav<S> {
  const same = options.same ?? ((a: S, b: S) => JSON.stringify(a) === JSON.stringify(b));
  const address = options.address;
  let current: S | null = null;
  /** The URL of a state, or undefined when the address is not kept: the URL then stays as it is. */
  const urlOf = (state: S): string | undefined => {
    if (!address) return undefined;
    const route = address.toRoute(state);
    if (route === null) return undefined;
    const { pathname, search } = address.location;
    return `${pathname}${search}${routeHash(route)}`;
  };
  const onPop = (event: { state: unknown }): void => {
    const entry = entryOf(event.state, options.isState);
    if (entry !== null) {
      current = entry.state;
      options.restore(entry.state);
      return;
    }
    // An entry that is not ours: a fragment typed or pasted into the address. A place in it is gone to, and the entry
    // becomes one of ours.
    if (!address) return;
    const state = placeOf(address);
    if (state === null) return;
    current = state;
    options.history.replaceState({ appkitNav: 1, state } satisfies Entry<S>, '', urlOf(state));
    options.restore(state);
  };
  options.window.addEventListener('popstate', onPop);
  return {
    record(state, recordOptions) {
      const entry: Entry<S> = { appkitNav: 1, state };
      // While a pairing code is in the address the URL is left to the pairing, which takes the code out.
      const url = address && hasPairing(address.location.hash) ? undefined : urlOf(state);
      if (current !== null && same(current, state)) {
        // The same place, but another part of the page changed the address (or gave it back): it follows the place
        // again, in the same entry.
        const { pathname, search, hash } = address?.location ?? { pathname: '', search: '', hash: '' };
        if (url !== undefined && url !== `${pathname}${search}${hash}`) options.history.replaceState(entry, '', url);
        return;
      }
      if (current === null || recordOptions?.replace === true) options.history.replaceState(entry, '', url);
      else options.history.pushState(entry, '', url);
      current = state;
    },
    stop() {
      options.window.removeEventListener('popstate', onPop);
    },
  };
}
