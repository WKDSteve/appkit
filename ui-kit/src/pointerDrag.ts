// Dragging an item with a finger: HTML5 drag does not start on a touch screen, so the grip of an item follows its own
// pointer (touch or pen) and the places to drop on are found by the point the finger is on. The mouse keeps the HTML5
// drag of the item; this is the way for a finger or a pen.

/** What the grip of an item tells the view. */
export interface PointerDragEvent {
  phase: 'start' | 'move' | 'end' | 'cancel';
  /** The item whose grip is held (a key the view chose: an id, or more than one in JSON). */
  id: string;
  x: number;
  y: number;
}

/** The part of a document the lookup needs (the real one has more). */
export interface PointSource {
  elementFromPoint(x: number, y: number): Element | null;
}

/** How far a finger goes from where it went down before the places count (a short wiggle is a tap, not a drag). */
export const GRAB = 24;

/** Whether a finger at `now` has gone `GRAB` or more from where it went down (`from`): only then a place counts. */
export function farEnough(from: { x: number; y: number }, now: { x: number; y: number }): boolean {
  return Math.hypot(now.x - from.x, now.y - from.y) >= GRAB;
}

/**
 * The value of `attribute` on the place under the point (the nearest element that carries it), or null when the point
 * is on no such place. An empty value is a value (the group of the items that have none), not "no place".
 */
export function placeAt(doc: PointSource, x: number, y: number, attribute: string): string | null {
  return doc.elementFromPoint(x, y)?.closest(`[${attribute}]`)?.getAttribute(attribute) ?? null;
}

/** A finger or a pen starts a drag by the grip; the mouse has the HTML5 drag of the item. */
export function startsOnGrip(pointerType: string): boolean {
  return pointerType === 'touch' || pointerType === 'pen';
}

/** The touch a grip is held by, from its `pointerdown` to the end of the drag. */
interface Held {
  pointerId: number;
  el: Element;
  x: number;
  y: number;
  /** The browser was told that this touch is no gesture of its own (the `touchstart` was taken). */
  taken: boolean;
  drop: () => void;
}

/**
 * The grip of an item: follows its own pointer from `start` to `end` and tells `sink` about it. The touch that holds
 * it is taken from the browser: a finger that lifts while it still moves (a flick) is the start of a fling to Chrome,
 * although the grip scrolls nothing (`touch-action: none`), and a tap while that fling runs only stops it: no click,
 * for up to about two seconds after the drop (measured 2026-11-18: 12 of 12 taps lost after a flick, none when the
 * finger rested before it lifted or the `touchstart` was taken). A `touchstart` that is taken makes no gesture at
 * all. It makes no click either, so a tap on the grip (the finger did not go `GRAB` far) is told as the click it would
 * have been: whatever listens for one (the card that selects, the hint popup) hears it as before. A pen sends no touch
 * events; its click is the browser's.
 */
export function createGrip(
  sink: () => ((event: PointerDragEvent) => void) | undefined,
  blocked: () => boolean,
): (e: PointerEvent, id: string, phase: PointerDragEvent['phase']) => void {
  // The item whose grip a finger holds: the pointer is followed on the grip (captured), wherever it goes.
  let gripped: string | null = null;
  const follow = (e: PointerEvent, id: string, phase: PointerDragEvent['phase']): void => {
    const on = sink();
    if (!on) return;
    if (phase === 'start') {
      if (!startsOnGrip(e.pointerType) || blocked()) return;
      e.preventDefault();
      try {
        (e.currentTarget as Element).setPointerCapture(e.pointerId);
      } catch {
        // A pointer that is gone cannot be captured; the events then stay on whatever is under the finger.
      }
      gripped = id;
    } else if (gripped !== id) {
      return;
    }
    if (phase === 'end' || phase === 'cancel') gripped = null;
    on({ phase, id, x: e.clientX, y: e.clientY });
  };

  let held: Held | null = null;
  return (e, id, phase) => {
    const el = e.currentTarget as Element | null;
    if (phase === 'start' && startsOnGrip(e.pointerType) && typeof el?.addEventListener === 'function') {
      held?.drop();
      // The `touchstart` of this touch comes after its `pointerdown`, to the same element.
      const take = (touch: Event): void => {
        if (!touch.cancelable) return;
        touch.preventDefault();
        now.taken = true;
      };
      const now: Held = {
        pointerId: e.pointerId,
        el,
        x: e.clientX,
        y: e.clientY,
        taken: false,
        drop: () => el.removeEventListener('touchstart', take),
      };
      el.addEventListener('touchstart', take, { passive: false, once: true });
      held = now;
    }
    follow(e, id, phase);
    // Only the finger that holds the grip ends its hold: a second finger that lifts elsewhere is not this touch.
    if ((phase === 'end' || phase === 'cancel') && held !== null && held.pointerId === e.pointerId) {
      const was = held;
      held = null;
      was.drop();
      if (phase === 'end' && was.taken && !farEnough(was, { x: e.clientX, y: e.clientY })) {
        was.el.dispatchEvent(
          new PointerEvent('click', {
            bubbles: true,
            cancelable: true,
            composed: true,
            pointerType: e.pointerType,
            pointerId: e.pointerId,
            clientX: e.clientX,
            clientY: e.clientY,
          }),
        );
      }
    }
  };
}
