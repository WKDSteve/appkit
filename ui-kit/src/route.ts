// The place of a page in its address: the fragment holds it as parameters, `#view=board&task=a%2Fb`. A place that is
// in the address can be opened in a new tab (a link), opened again with a reload, and copied.
//
// The fragment, never the path or the query: the hub serves the page at `/` alone (any other path is a 404), the web
// variant is one file opened from disk, and a fragment never goes to a server or into a `Referer`. It ends up in the
// browser's history, as a pairing link does. `pair` is the pairing code's (`pair.ts`): a route never reads or writes it.
//
// The kit knows no view and no item: the app says which keys it has and what their values may be.

/** The parameters of a place. A key is a short lower-case word; a value is one line of printable text. */
export type Route = Readonly<Record<string, string>>;

/** The longest value a route takes (an id is an area and a name; nothing real is near this). */
export const MAX_VALUE = 300;

const KEY = /^[a-z][a-z0-9_-]{0,31}$/;
const PAIR = 'pair';

function valueOk(value: string): boolean {
  return value !== '' && value.length <= MAX_VALUE && !/[\u0000-\u001f\u007f]/.test(value);
}

/**
 * The route a fragment holds: each `key=value` part whose key is a route key and whose value is a value. The first part
 * of a key wins; a part that is not one (no `=`, a broken escape, the pairing code) is left out.
 */
export function readRoute(hash: string): Record<string, string> {
  const text = hash.startsWith('#') ? hash.slice(1) : hash;
  const route: Record<string, string> = {};
  for (const part of text.split('&')) {
    const at = part.indexOf('=');
    if (at < 0) continue;
    const key = part.slice(0, at);
    if (!KEY.test(key) || key === PAIR || Object.hasOwn(route, key)) continue;
    let value: string;
    try {
      value = decodeURIComponent(part.slice(at + 1)).trim();
    } catch {
      continue;
    }
    if (valueOk(value)) route[key] = value;
  }
  return route;
}

/**
 * The fragment of a route, `#key=value&...` in the order of its keys, or empty for none. A key a route cannot hold is
 * an error of the app: it throws. A value comes from the data (the id of an item from its file) and is left out when a
 * route cannot carry it, or would not read it back the same (space at its ends): the address then names less, the page
 * still works.
 */
export function routeHash(route: Route): string {
  const parts: string[] = [];
  for (const [key, value] of Object.entries(route)) {
    if (!KEY.test(key) || key === PAIR) throw new Error(`appkit route: not a route key: ${JSON.stringify(key)}`);
    if (!valueOk(value) || value.trim() !== value) continue;
    parts.push(`${key}=${encodeURIComponent(value)}`);
  }
  return parts.length === 0 ? '' : `#${parts.join('&')}`;
}

/** Where a route is in this page: its path and query, and the route as the fragment. What a link's `href` is. */
export function routeHref(location: { pathname: string; search: string }, route: Route): string {
  return `${location.pathname}${location.search}${routeHash(route)}`;
}

/** Does the fragment carry a pairing code? Such an address belongs to the pairing, not to the route. */
export function hasPairing(hash: string): boolean {
  const text = hash.startsWith('#') ? hash.slice(1) : hash;
  return text.split('&').some((part) => part.split('=')[0] === PAIR);
}
