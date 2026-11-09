import { describe, expect, it } from 'vitest';
import { createGrip, farEnough, GRAB, placeAt, startsOnGrip, type PointerDragEvent } from './pointerDrag';

describe('the place of a view under a finger', () => {
  /** An element inside a place that carries `attribute` with `value` (null: no such place around it). */
  const inside = (attribute: string, value: string | null): Element =>
    ({
      closest: (sel: string) => (sel === `[${attribute}]` && value !== null ? { getAttribute: (n: string) => (n === attribute ? value : null) } : null),
    }) as unknown as Element;

  it('is the value of the attribute of the place the point is in, the empty one too (the group of none)', () => {
    expect(placeAt({ elementFromPoint: () => inside('data-group', 'doing') }, 0, 0, 'data-group')).toBe('doing');
    expect(placeAt({ elementFromPoint: () => inside('data-group', '') }, 0, 0, 'data-group')).toBe('');
    expect(placeAt({ elementFromPoint: () => inside('data-cell', '["1","open"]') }, 0, 0, 'data-cell')).toBe('["1","open"]');
  });

  it('is none off every place, and off the page', () => {
    expect(placeAt({ elementFromPoint: () => inside('data-group', null) }, 0, 0, 'data-group')).toBeNull();
    expect(placeAt({ elementFromPoint: () => null }, 0, 0, 'data-group')).toBeNull();
  });
});

describe('how far a finger goes before a place counts', () => {
  it('is GRAB from where it went down, in any direction; a wiggle is less', () => {
    expect(farEnough({ x: 0, y: 0 }, { x: 6, y: 6 })).toBe(false);
    expect(farEnough({ x: 10, y: 10 }, { x: 10 - GRAB, y: 10 })).toBe(true);
    expect(farEnough({ x: 0, y: 0 }, { x: 0, y: GRAB - 1 })).toBe(false);
    expect(farEnough({ x: 0, y: 0 }, { x: 17, y: 17 })).toBe(true);
  });
});

describe('who drags by the grip', () => {
  it('is a finger or a pen, not the mouse', () => {
    expect(startsOnGrip('touch')).toBe(true);
    expect(startsOnGrip('pen')).toBe(true);
    expect(startsOnGrip('mouse')).toBe(false);
  });
});

/** A pointer event as far as the grip reads it. */
function pointer(type: string, x = 0, y = 0) {
  const captured: number[] = [];
  let prevented = false;
  const e = {
    pointerType: type,
    pointerId: 7,
    clientX: x,
    clientY: y,
    preventDefault: () => (prevented = true),
    currentTarget: { setPointerCapture: (id: number) => captured.push(id) },
  };
  return { e: e as unknown as PointerEvent, captured, prevented: () => prevented };
}

describe('the grip of a row', () => {
  it('follows a finger from the start to the end, and only the row it started on', () => {
    const seen: PointerDragEvent[] = [];
    const grip = createGrip(() => (ev) => seen.push(ev), () => false);
    const down = pointer('touch', 1, 2);
    grip(down.e, 'a', 'start');
    expect(down.captured).toEqual([7]);
    expect(down.prevented()).toBe(true);
    grip(pointer('touch', 5, 6).e, 'b', 'move');
    grip(pointer('touch', 3, 4).e, 'a', 'move');
    grip(pointer('touch', 3, 4).e, 'a', 'end');
    grip(pointer('touch', 9, 9).e, 'a', 'move');
    expect(seen.map((x) => [x.phase, x.id, x.x, x.y])).toEqual([
      ['start', 'a', 1, 2],
      ['move', 'a', 3, 4],
      ['end', 'a', 3, 4],
    ]);
  });

  it('is not started by a mouse, nor while a write is under way, nor without a place to send to', () => {
    const seen: PointerDragEvent[] = [];
    let busy = false;
    let sink: ((ev: PointerDragEvent) => void) | undefined = (ev) => seen.push(ev);
    const grip = createGrip(() => sink, () => busy);
    grip(pointer('mouse').e, 'a', 'start');
    busy = true;
    grip(pointer('touch').e, 'a', 'start');
    busy = false;
    sink = undefined;
    grip(pointer('touch').e, 'a', 'start');
    expect(seen).toEqual([]);
  });

  it('goes on when the pointer cannot be captured', () => {
    const seen: PointerDragEvent[] = [];
    const grip = createGrip(() => (ev) => seen.push(ev), () => false);
    const down = pointer('pen');
    (down.e.currentTarget as unknown as { setPointerCapture: () => void }).setPointerCapture = () => {
      throw new Error('gone');
    };
    grip(down.e, 'a', 'start');
    expect(seen).toHaveLength(1);
  });
});
