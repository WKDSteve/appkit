// Where the person put a float (a dialog over the page: the task, the capture, the check ...) and how big they made it.
// One place per kind of float, kept in this browser. A float that was never moved has no entry and stands where the
// browser puts a dialog (in the middle); one that was only moved keeps the size its content gives it.

/** A box on the screen, in CSS pixels from the top left corner of the viewport. */
export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The size of the viewport. */
export interface Viewport {
  w: number;
  h: number;
}

/** What is kept of one float: where its top left corner is, and its size once it was given one. */
export interface FloatPlace {
  x: number;
  y: number;
  w?: number;
  h?: number;
}
export type FloatPlaces = Record<string, FloatPlace>;

/** How far a float stays from the edges of the viewport, in pixels. */
export const FLOAT_MARGIN = 8;
/** The smallest a float may be made (a viewport smaller than that gets what it has). */
export const FLOAT_MIN = { w: 240, h: 120 };
/** How far one press of an arrow key moves a float or one of its edges. */
export const FLOAT_STEP = 16;

const round = (v: number) => Math.round(v);

/**
 * The box moved and shrunk so that all of it lies inside the viewport, with `FLOAT_MARGIN` around it. A box larger
 * than the viewport is made as large as it allows; one smaller than `FLOAT_MIN` grows to it (where the viewport
 * allows that much).
 */
export function clampBox(box: Box, view: Viewport): Box {
  const m = FLOAT_MARGIN;
  const roomW = Math.max(0, view.w - 2 * m);
  const roomH = Math.max(0, view.h - 2 * m);
  const w = Math.min(roomW, Math.max(Math.min(FLOAT_MIN.w, roomW), box.w));
  const h = Math.min(roomH, Math.max(Math.min(FLOAT_MIN.h, roomH), box.h));
  const x = Math.min(view.w - m - w, Math.max(m, box.x));
  const y = Math.min(view.h - m - h, Math.max(m, box.y));
  return { x: round(x), y: round(y), w: round(w), h: round(h) };
}

/** The box a float has after `dx`/`dy` of a drag of its head (its size stays), inside the viewport. */
export function movedBox(start: Box, dx: number, dy: number, view: Viewport): Box {
  return clampBox({ ...start, x: start.x + dx, y: start.y + dy }, view);
}

/** The box a float has after `dx`/`dy` of a drag of its lower right corner (its top left corner stays). */
export function sizedBox(start: Box, dx: number, dy: number, view: Viewport): Box {
  const m = FLOAT_MARGIN;
  const w = Math.min(view.w - m - start.x, start.w + dx);
  const h = Math.min(view.h - m - start.y, start.h + dy);
  return clampBox({ ...start, w, h }, view);
}

/**
 * The keyboard: an arrow key moves the float by `FLOAT_STEP`, with Shift it moves the lower right corner (the float
 * gets wider, narrower, taller or shorter). Another key gives `null`. `resized` says whether the size changed.
 */
export function keyedBox(box: Box, key: string, shift: boolean, view: Viewport): { box: Box; resized: boolean } | null {
  const steps: Record<string, [number, number]> = {
    ArrowRight: [FLOAT_STEP, 0],
    ArrowLeft: [-FLOAT_STEP, 0],
    ArrowDown: [0, FLOAT_STEP],
    ArrowUp: [0, -FLOAT_STEP],
  };
  const d = steps[key];
  if (!d) return null;
  return shift
    ? { box: sizedBox(box, d[0], d[1], view), resized: true }
    : { box: movedBox(box, d[0], d[1], view), resized: false };
}

const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

/** The places kept in the browser; an entry that is not numbers is left out (a broken text gives none). */
export function parseFloats(text: string | null): FloatPlaces {
  let raw: unknown;
  try {
    raw = text ? JSON.parse(text) : null;
  } catch {
    return {};
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return {};
  const out: FloatPlaces = {};
  for (const [kind, v] of Object.entries(raw)) {
    if (typeof v !== 'object' || v === null) continue;
    const { x, y, w, h } = v as Record<string, unknown>;
    if (!finite(x) || !finite(y)) continue;
    const place: FloatPlace = { x: round(x), y: round(y) };
    // A size is kept only whole: a width without its height is no size.
    if (finite(w) && finite(h) && w > 0 && h > 0) {
      place.w = round(w);
      place.h = round(h);
    }
    out[kind] = place;
  }
  return out;
}

export function serializeFloats(places: FloatPlaces): string {
  return JSON.stringify(places);
}

/** The places with the one of `kind` set, or taken away with `null`. */
export function withPlace(places: FloatPlaces, kind: string, place: FloatPlace | null): FloatPlaces {
  const { [kind]: _gone, ...rest } = places;
  return place === null ? rest : { ...rest, [kind]: place };
}

/** The part of the browser's storage the places need. */
export interface PlaceStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** The place kept for `kind`, or null (none, or a storage the browser refuses). */
export function loadPlace(store: PlaceStore | null, key: string, kind: string): FloatPlace | null {
  try {
    return parseFloats(store?.getItem(key) ?? null)[kind] ?? null;
  } catch {
    return null;
  }
}

/** Keep the place of `kind` (or forget it with `null`). A storage the browser refuses keeps nothing, and says nothing. */
export function savePlace(store: PlaceStore | null, key: string, kind: string, place: FloatPlace | null): void {
  if (!store) return;
  try {
    const places = parseFloats(store.getItem(key));
    store.setItem(key, serializeFloats(withPlace(places, kind, place)));
  } catch {
    // Private mode or blocked site data: the float works, it just does not remember.
  }
}
