// @vitest-environment jsdom
// A submenu of the menu bar, in a DOM: its ARIA, the keys of the ARIA menu pattern, the pointer (rest, corridor,
// click) and the narrow window, where it folds open inside its menu.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import MenuBar from './MenuBar.svelte';
import { SUBMENU_GRACE_MS, SUBMENU_OPEN_MS, type Menu } from './menu';

const picked: string[] = [];
const opened: string[] = [];
const radio = (label: string, checked: boolean, extra: Record<string, unknown> = {}) => ({
  kind: 'radio' as const,
  label,
  checked,
  onselect: () => picked.push(label),
  ...extra,
});
const menus: Menu[] = [
  {
    id: 'view',
    label: 'View',
    items: [
      radio('Today', false),
      { kind: 'section', label: 'Views' },
      {
        kind: 'submenu',
        label: 'Plans',
        hint: 'Learning plans',
        items: [
          radio('Map', true, { href: '/#view=pathmap', onopennew: () => opened.push('Map') }),
          radio('Paths', false),
          { kind: 'submenu', label: 'Later', items: [radio('Rewards', false)] },
        ],
      },
      radio('Repos', false),
      { kind: 'submenu', label: 'Empty', items: [{ kind: 'info', label: 'nothing yet' }] },
    ],
  },
  { id: 'project', label: 'Project', items: [{ kind: 'action', label: 'New', onselect: () => picked.push('New') }] },
];

let app: ReturnType<typeof mount> | undefined;
const start = (width = 1024): void => {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });
  document.body.innerHTML = '';
  app = mount(MenuBar, {
    target: document.body,
    props: { menus, label: 'Menu', labels: { submenuCurrent: '{label}, now: {current}' } },
  });
  flushSync();
};
beforeEach(() => {
  picked.length = 0;
  opened.length = 0;
  start();
});
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

const settle = async (): Promise<void> => {
  await tick();
  await tick();
};
const key = (target: EventTarget, k: string, init: KeyboardEventInit = {}): KeyboardEvent => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
};
const trigger = (id: string) => document.querySelector<HTMLButtonElement>(`button.mb-trigger[data-for="${id}"]`)!;
const item = (label: string) =>
  [...document.querySelectorAll<HTMLElement>('.mb-item')].find(
    (el) => el.querySelector('.mb-text')?.textContent === label,
  )!;
const parent = () => item('Plans');
const sub = () => document.querySelector<HTMLElement>('[data-sub]');
const active = () => document.activeElement?.querySelector('.mb-text')?.textContent ?? null;
const pointer = (target: EventTarget, type: string, init: PointerEventInit = {}): void => {
  target.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerType: 'mouse', ...init }));
  flushSync();
};

async function openView(): Promise<void> {
  key(trigger('view'), 'ArrowDown');
  await settle();
}

async function toParent(): Promise<void> {
  await openView();
  while (active() !== 'Plans') key(document.activeElement!, 'ArrowDown');
}

describe('the item of a submenu', () => {
  it('is a menuitem with a popup, closed; its name and its dot say which item inside is checked', async () => {
    await openView();
    const p = parent();
    expect(p.getAttribute('role')).toBe('menuitem');
    expect(p.getAttribute('aria-haspopup')).toBe('menu');
    expect(p.getAttribute('aria-expanded')).toBe('false');
    expect(p.getAttribute('aria-label')).toBe('Plans, now: Map');
    expect(p.querySelector('.mb-mark')?.textContent).toBe('●');
    expect(p.querySelector('.mb-arrow')?.textContent).toBe('▸');
    expect(p.hasAttribute('aria-checked')).toBe(false);
    expect(sub()).toBeNull();
  });

  it('is disabled when nothing inside can be chosen', async () => {
    await openView();
    expect((item('Empty') as HTMLButtonElement).disabled).toBe(true);
  });

  it('carries its hint while closed, and not while its list is open', async () => {
    await toParent();
    expect(parent().getAttribute('data-hint')).toBe('Learning plans');
    key(parent(), 'ArrowRight');
    await settle();
    expect(parent().hasAttribute('data-hint')).toBe(false);
  });
});

describe('the keys of a submenu', () => {
  it('opens with Right onto its first item, a menu named by its label beside the menu', async () => {
    await toParent();
    const e = key(parent(), 'ArrowRight');
    await settle();
    expect(e.defaultPrevented).toBe(true);
    expect(parent().getAttribute('aria-expanded')).toBe('true');
    expect(sub()?.getAttribute('role')).toBe('menu');
    expect(sub()?.getAttribute('aria-label')).toBe('Plans');
    expect(sub()?.classList.contains('mb-fly')).toBe(true);
    expect(sub()?.dataset.side).toBe('right');
    expect(parent().parentElement?.getAttribute('role')).toBe('none');
    expect(active()).toBe('Map');
  });

  it('opens with Enter and with Space too', async () => {
    await toParent();
    key(parent(), 'Enter');
    await settle();
    expect(active()).toBe('Map');
    key(document.activeElement!, 'ArrowLeft');
    await settle();
    key(parent(), ' ');
    await settle();
    expect(active()).toBe('Map');
  });

  it('walks its items with the arrows, Home, End and a letter, and stays inside', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    key(document.activeElement!, 'ArrowDown');
    expect(active()).toBe('Paths');
    key(document.activeElement!, 'End');
    expect(active()).toBe('Rewards');
    key(document.activeElement!, 'ArrowDown');
    expect(active()).toBe('Map');
    key(document.activeElement!, 'ArrowUp');
    expect(active()).toBe('Rewards');
    key(document.activeElement!, 'Home');
    expect(active()).toBe('Map');
    key(document.activeElement!, 'p');
    expect(active()).toBe('Paths');
  });

  it('shows a submenu inside it flat, under a heading of its label', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    expect(sub()?.querySelector('.mb-section')?.textContent).toBe('Later');
    expect(sub()?.querySelectorAll('[data-sub]').length).toBe(0);
    expect(sub()?.querySelectorAll('[aria-haspopup]').length).toBe(0);
  });

  it('closes with Left onto its item, and the menu stays open', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    key(document.activeElement!, 'ArrowLeft');
    await settle();
    expect(sub()).toBeNull();
    expect(document.activeElement).toBe(parent());
    expect(parent().getAttribute('aria-expanded')).toBe('false');
    expect(document.querySelector('[data-menu="view"]')).not.toBeNull();
  });

  it('closes with Escape onto its item; a second Escape closes the menu onto its trigger', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    const first = key(document.activeElement!, 'Escape');
    await settle();
    expect(first.defaultPrevented).toBe(true);
    expect(sub()).toBeNull();
    expect(document.activeElement).toBe(parent());
    key(document.activeElement!, 'Escape');
    await settle();
    expect(document.querySelector('[data-menu="view"]')).toBeNull();
    expect(document.activeElement).toBe(trigger('view'));
  });

  it('goes on with Right on one of its items to the next menu of the bar', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    key(document.activeElement!, 'ArrowDown');
    key(document.activeElement!, 'ArrowRight');
    await settle();
    expect(document.querySelector('[data-menu="view"]')).toBeNull();
    expect(sub()).toBeNull();
    expect(active()).toBe('New');
  });

  it('closes with Right in a bar of one menu, which opens that menu again on its first item (also folded)', async () => {
    await (app && unmount(app));
    document.body.innerHTML = '';
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 380 });
    app = mount(MenuBar, { target: document.body, props: { menus: [menus[0]!], label: 'Menu' } });
    flushSync();
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    key(document.activeElement!, 'ArrowRight');
    await settle();
    expect(sub()).toBeNull();
    expect(active()).toBe('Today');
  });

  it('is passed over by the arrows of the menu, and closes when the focus goes to another item', async () => {
    await toParent();
    parent().click();
    await settle();
    expect(sub()).not.toBeNull();
    key(parent(), 'ArrowDown');
    await settle();
    expect(active()).toBe('Repos');
    expect(sub()).toBeNull();
  });

  it('chooses an item with Enter (a click) and closes the whole menu', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    key(document.activeElement!, 'ArrowDown');
    (document.activeElement as HTMLElement).click();
    await settle();
    expect(picked).toEqual(['Paths']);
    expect(document.querySelector('[data-menu="view"]')).toBeNull();
  });
});

describe('a link in a submenu', () => {
  it('is a link to its place, and opens apart with Ctrl+Enter and a middle click', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    const map = item('Map');
    expect(map.tagName).toBe('A');
    expect(map.getAttribute('href')).toBe('/#view=pathmap');
    const e = key(map, 'Enter', { ctrlKey: true });
    await settle();
    expect(e.defaultPrevented).toBe(true);
    expect(opened).toEqual(['Map']);
    expect(picked).toEqual([]);
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    const aux = new MouseEvent('auxclick', { bubbles: true, cancelable: true, button: 1 });
    item('Map').dispatchEvent(aux);
    expect(aux.defaultPrevented).toBe(true);
    expect(opened).toEqual(['Map', 'Map']);
  });

  it('goes to its place in the page with a plain click', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    const click = new MouseEvent('click', { bubbles: true, cancelable: true });
    item('Map').dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(picked).toEqual(['Map']);
  });
});

describe('the pointer on a submenu', () => {
  it('opens it after a short rest on its item, without taking the focus', async () => {
    await openView();
    vi.useFakeTimers();
    pointer(parent(), 'pointermove', { clientX: 50, clientY: 100 });
    vi.advanceTimersByTime(SUBMENU_OPEN_MS - 10);
    flushSync();
    expect(sub()).toBeNull();
    vi.advanceTimersByTime(20);
    flushSync();
    expect(sub()).not.toBeNull();
    expect(active()).toBe('Today');
  });

  it('does not open when the pointer leaves first, nor for a finger', async () => {
    await openView();
    vi.useFakeTimers();
    pointer(parent(), 'pointermove', { clientX: 50, clientY: 100 });
    pointer(parent(), 'pointerleave');
    vi.advanceTimersByTime(SUBMENU_OPEN_MS * 2);
    flushSync();
    expect(sub()).toBeNull();
    pointer(parent(), 'pointermove', { pointerType: 'touch' });
    vi.advanceTimersByTime(SUBMENU_OPEN_MS * 2);
    flushSync();
    expect(sub()).toBeNull();
  });

  it('opens it at once on a click', async () => {
    await openView();
    parent().click();
    flushSync();
    expect(sub()).not.toBeNull();
    parent().click();
    flushSync();
    expect(sub()).not.toBeNull(); // beside the menu a click only opens
  });

  it('closes it when the pointer goes to a neighbour, and keeps it while it is on its way there', async () => {
    await openView();
    parent().click();
    flushSync();
    // The submenu lies right of its item: x 300..500, y 100..300.
    vi.spyOn(sub()!, 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({ x: 300, y: 100, width: 200, height: 200 }));
    vi.useFakeTimers();
    pointer(parent(), 'pointermove', { clientX: 200, clientY: 110 });
    // Down and to the right over the next item, toward the submenu: it stays for a while.
    pointer(item('Repos'), 'pointermove', { clientX: 260, clientY: 150 });
    expect(sub()).not.toBeNull();
    // It arrives: the submenu stays.
    pointer(sub()!, 'pointerenter');
    vi.advanceTimersByTime(SUBMENU_GRACE_MS * 2);
    flushSync();
    expect(sub()).not.toBeNull();
    // Back on the item, then straight down to the neighbour: closed at once.
    pointer(parent(), 'pointermove', { clientX: 200, clientY: 110 });
    pointer(item('Repos'), 'pointermove', { clientX: 202, clientY: 140 });
    expect(sub()).toBeNull();
  });

  it('counts the grace from the last move, not from the leave, and closes at once on a turn away', async () => {
    await openView();
    parent().click();
    flushSync();
    vi.spyOn(sub()!, 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({ x: 300, y: 100, width: 200, height: 200 }));
    vi.useFakeTimers();
    pointer(parent(), 'pointermove', { clientX: 200, clientY: 110 });
    pointer(item('Repos'), 'pointermove', { clientX: 230, clientY: 125 });
    vi.advanceTimersByTime(SUBMENU_GRACE_MS - 50);
    // An unhurried diagonal: a later move on the way starts the grace again (the tip stayed where the pointer left).
    pointer(item('Repos'), 'pointermove', { clientX: 260, clientY: 150 });
    vi.advanceTimersByTime(SUBMENU_GRACE_MS - 50);
    flushSync();
    expect(sub()).not.toBeNull();
    // A turn back, away from the submenu: closed at once.
    pointer(item('Repos'), 'pointermove', { clientX: 255, clientY: 152 });
    expect(sub()).toBeNull();
  });

  it('closes it after the grace when the pointer heads for it but does not arrive', async () => {
    await openView();
    parent().click();
    flushSync();
    vi.spyOn(sub()!, 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({ x: 300, y: 100, width: 200, height: 200 }));
    vi.useFakeTimers();
    pointer(parent(), 'pointermove', { clientX: 200, clientY: 110 });
    pointer(item('Repos'), 'pointermove', { clientX: 260, clientY: 150 });
    vi.advanceTimersByTime(SUBMENU_GRACE_MS + 10);
    flushSync();
    expect(sub()).toBeNull();
  });
});

describe('a submenu in a narrow window', () => {
  it('folds open inside its menu, indented and marked ▾, and folds shut on a click', async () => {
    await (app && unmount(app));
    start(380);
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    const list = sub()!;
    expect(list.classList.contains('mb-fold')).toBe(true);
    expect(list.hasAttribute('popover')).toBe(false);
    expect(list.closest('[data-menu="view"]')).not.toBeNull();
    expect(parent().querySelector('.mb-arrow')?.textContent).toBe('▾');
    expect(active()).toBe('Map');
    // The arrows of the menu still pass it over.
    key(parent(), 'ArrowDown');
    expect(active()).toBe('Repos');
    parent().dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));
    flushSync();
    expect(sub()).toBeNull();
    parent().dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));
    flushSync();
    expect(sub()?.classList.contains('mb-fold')).toBe(true);
  });

  it('does not open on a resting pointer (the menu would jump under it)', async () => {
    await (app && unmount(app));
    start(380);
    await openView();
    vi.useFakeTimers();
    pointer(parent(), 'pointermove', { clientX: 50, clientY: 100 });
    vi.advanceTimersByTime(SUBMENU_OPEN_MS * 2);
    flushSync();
    expect(sub()).toBeNull();
  });

  it('changes between beside and inside when the window changes its width', async () => {
    await toParent();
    key(parent(), 'ArrowRight');
    await settle();
    expect(sub()?.classList.contains('mb-fly')).toBe(true);
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 380 });
    window.dispatchEvent(new Event('resize'));
    flushSync();
    expect(sub()?.classList.contains('mb-fold')).toBe(true);
  });
});
