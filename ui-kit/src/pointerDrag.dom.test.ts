// @vitest-environment jsdom
// The grip of an item with a real element: the touch that holds it is taken from the browser (no gesture, so no fling
// whose stop would swallow the next tap), and a tap on the grip is still heard as a click.
import { afterEach, describe, expect, it } from 'vitest';
import { createGrip, GRAB, type PointerDragEvent } from './pointerDrag';

afterEach(() => document.body.replaceChildren());

function gripOn(blocked = false) {
  const el = document.createElement('span');
  document.body.append(el);
  const seen: PointerDragEvent[] = [];
  const clicks: PointerEvent[] = [];
  el.addEventListener('click', (e) => clicks.push(e as PointerEvent));
  const grip = createGrip(
    () => (ev) => seen.push(ev),
    () => blocked,
  );
  const pointer = (pointerType: string, x: number, y: number, pointerId = 7): PointerEvent =>
    ({ pointerType, pointerId, clientX: x, clientY: y, currentTarget: el, preventDefault: () => undefined }) as unknown as PointerEvent;
  /** The `touchstart` the browser sends after the `pointerdown` of a finger; whether the page took it. */
  const touchstart = (): boolean => {
    const touch = new Event('touchstart', { bubbles: true, cancelable: true });
    el.dispatchEvent(touch);
    return touch.defaultPrevented;
  };
  return { grip, pointer, touchstart, seen, clicks };
}

describe('the grip, held by a finger', () => {
  it('takes the touchstart of its own touch, once', () => {
    const g = gripOn();
    g.grip(g.pointer('touch', 10, 10), 'a', 'start');
    expect(g.touchstart()).toBe(true);
    // A second finger on the grip is not this drag.
    expect(g.touchstart()).toBe(false);
  });

  it('leaves the touch alone before a finger is down on it and after the drag', () => {
    const g = gripOn();
    expect(g.touchstart()).toBe(false);
    g.grip(g.pointer('touch', 10, 10), 'a', 'start');
    g.grip(g.pointer('touch', 10, 10 + GRAB), 'a', 'end');
    expect(g.touchstart()).toBe(false);
  });

  it('tells a tap as the click the taken touch no longer makes, and a drag as none', () => {
    const g = gripOn();
    g.grip(g.pointer('touch', 10, 10), 'a', 'start');
    g.touchstart();
    g.grip(g.pointer('touch', 12, 11), 'a', 'end');
    expect(g.clicks).toHaveLength(1);
    expect(g.clicks[0]?.pointerType).toBe('touch');
    expect(g.clicks[0]?.bubbles).toBe(true);

    g.grip(g.pointer('touch', 10, 10), 'a', 'start');
    g.touchstart();
    g.grip(g.pointer('touch', 10 + GRAB, 10), 'a', 'move');
    g.grip(g.pointer('touch', 10 + GRAB, 10), 'a', 'end');
    g.grip(g.pointer('touch', 10, 10), 'a', 'start');
    g.touchstart();
    g.grip(g.pointer('touch', 10, 10), 'a', 'cancel');
    expect(g.clicks).toHaveLength(1);
  });

  it('keeps the hold of its finger when another finger lifts elsewhere', () => {
    const g = gripOn();
    g.grip(g.pointer('touch', 10, 10, 7), 'a', 'start');
    g.touchstart();
    // A second finger ends on another grip of the view: not this touch.
    g.grip(g.pointer('touch', 200, 200, 8), 'b', 'end');
    g.grip(g.pointer('touch', 11, 10, 7), 'a', 'end');
    expect(g.clicks).toHaveLength(1);
  });

  it('still tells the tap while a write is under way: the drag does not start, the click is the same', () => {
    const g = gripOn(true);
    g.grip(g.pointer('touch', 10, 10), 'a', 'start');
    expect(g.touchstart()).toBe(true);
    g.grip(g.pointer('touch', 10, 10), 'a', 'end');
    expect(g.seen).toEqual([]);
    expect(g.clicks).toHaveLength(1);
  });

  it('makes no click of its own for a pen (the browser makes it) and none for a mouse', () => {
    const g = gripOn();
    // A pen sends no touchstart: nothing was taken, so the click is left to the browser.
    g.grip(g.pointer('pen', 10, 10), 'a', 'start');
    g.grip(g.pointer('pen', 10, 10), 'a', 'end');
    expect(g.touchstart()).toBe(false);
    g.grip(g.pointer('mouse', 10, 10), 'a', 'start');
    expect(g.touchstart()).toBe(false);
    g.grip(g.pointer('mouse', 10, 10), 'a', 'end');
    expect(g.clicks).toEqual([]);
  });
});
