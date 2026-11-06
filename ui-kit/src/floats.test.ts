import { describe, expect, it } from 'vitest';
import {
  clampBox,
  FLOAT_MARGIN,
  FLOAT_STEP,
  keyedBox,
  loadPlace,
  movedBox,
  parseFloats,
  savePlace,
  serializeFloats,
  sizedBox,
  withPlace,
  type PlaceStore,
} from './floats';

const view = { w: 1000, h: 700 };

describe('a float in the viewport', () => {
  it('stays where it is when all of it is inside', () => {
    expect(clampBox({ x: 100, y: 50, w: 400, h: 300 }, view)).toEqual({ x: 100, y: 50, w: 400, h: 300 });
  });

  it('is pushed back in at every edge, with the margin', () => {
    expect(clampBox({ x: -50, y: -20, w: 400, h: 300 }, view)).toEqual({ x: FLOAT_MARGIN, y: FLOAT_MARGIN, w: 400, h: 300 });
    expect(clampBox({ x: 900, y: 600, w: 400, h: 300 }, view)).toEqual({ x: 1000 - 8 - 400, y: 700 - 8 - 300, w: 400, h: 300 });
  });

  it('is made as large as the viewport allows, and no smaller than its least size where there is room', () => {
    expect(clampBox({ x: 0, y: 0, w: 3000, h: 2000 }, view)).toEqual({ x: 8, y: 8, w: 984, h: 684 });
    expect(clampBox({ x: 10, y: 10, w: 20, h: 10 }, view)).toEqual({ x: 10, y: 10, w: 240, h: 120 });
    // A phone narrower than the least size: the float takes what there is.
    expect(clampBox({ x: 0, y: 0, w: 1200, h: 900 }, { w: 200, h: 100 })).toEqual({ x: 8, y: 8, w: 184, h: 84 });
  });

  it('moves with a drag of its head and keeps its size; its corner moves with a drag of the grip', () => {
    const start = { x: 100, y: 100, w: 300, h: 200 };
    expect(movedBox(start, 50, -30, view)).toEqual({ x: 150, y: 70, w: 300, h: 200 });
    expect(movedBox(start, 5000, 5000, view)).toEqual({ x: 692, y: 492, w: 300, h: 200 });
    expect(sizedBox(start, 40, 60, view)).toEqual({ x: 100, y: 100, w: 340, h: 260 });
    // The corner stops at the edge of the viewport; the upper left corner stays.
    expect(sizedBox(start, 5000, 5000, view)).toEqual({ x: 100, y: 100, w: 892, h: 592 });
    expect(sizedBox(start, -1000, -1000, view)).toEqual({ x: 100, y: 100, w: 240, h: 120 });
  });

  it('moves by the arrow keys, and changes its size with Shift; other keys do nothing', () => {
    const box = { x: 100, y: 100, w: 300, h: 200 };
    expect(keyedBox(box, 'ArrowRight', false, view)).toEqual({ box: { ...box, x: 100 + FLOAT_STEP }, resized: false });
    expect(keyedBox(box, 'ArrowUp', false, view)).toEqual({ box: { ...box, y: 100 - FLOAT_STEP }, resized: false });
    expect(keyedBox(box, 'ArrowLeft', true, view)).toEqual({ box: { ...box, w: 300 - FLOAT_STEP }, resized: true });
    expect(keyedBox(box, 'ArrowDown', true, view)).toEqual({ box: { ...box, h: 200 + FLOAT_STEP }, resized: true });
    expect(keyedBox(box, 'Enter', false, view)).toBeNull();
  });
});

/** The key an app keeps its places under. */
const KEY = 'an-app.floats';

describe('the kept places', () => {
  it('are read back as they were written', () => {
    const places = withPlace(withPlace({}, 'task', { x: 10, y: 20, w: 300, h: 200 }), 'capture', { x: 5, y: 6 });
    expect(parseFloats(serializeFloats(places))).toEqual(places);
    expect(withPlace(places, 'task', null)).toEqual({ capture: { x: 5, y: 6 } });
  });

  it('leave out what is not numbers, and a size that is not whole', () => {
    expect(parseFloats(null)).toEqual({});
    expect(parseFloats('not json')).toEqual({});
    expect(parseFloats('[1,2]')).toEqual({});
    expect(
      parseFloats(
        JSON.stringify({ a: { x: 'one', y: 2 }, b: { x: 1.4, y: 2.6, w: 300 }, c: { x: 1, y: 2, w: 300, h: -1 }, d: null, e: { x: 1, y: 2, w: 300, h: 200 } }),
      ),
    ).toEqual({ b: { x: 1, y: 3 }, c: { x: 1, y: 2 }, e: { x: 1, y: 2, w: 300, h: 200 } });
  });

  it('are kept in the storage per kind, and a storage that refuses keeps nothing and breaks nothing', () => {
    const map = new Map<string, string>();
    const store: PlaceStore = { getItem: (k) => map.get(k) ?? null, setItem: (k, v) => void map.set(k, v) };
    savePlace(store, KEY, 'task', { x: 1, y: 2 });
    savePlace(store, KEY, 'capture', { x: 3, y: 4, w: 300, h: 200 });
    expect(loadPlace(store, KEY, 'task')).toEqual({ x: 1, y: 2 });
    expect(JSON.parse(map.get(KEY) ?? '{}')).toEqual({ task: { x: 1, y: 2 }, capture: { x: 3, y: 4, w: 300, h: 200 } });
    savePlace(store, KEY, 'task', null);
    expect(loadPlace(store, KEY, 'task')).toBeNull();
    expect(loadPlace(store, KEY, 'capture')).toEqual({ x: 3, y: 4, w: 300, h: 200 });

    const refusing: PlaceStore = {
      getItem: () => {
        throw new Error('SecurityError');
      },
      setItem: () => {
        throw new Error('QuotaExceededError');
      },
    };
    expect(() => savePlace(refusing, KEY, 'task', { x: 1, y: 2 })).not.toThrow();
    expect(loadPlace(refusing, KEY, 'task')).toBeNull();
    expect(loadPlace(null, KEY, 'task')).toBeNull();
  });
});
