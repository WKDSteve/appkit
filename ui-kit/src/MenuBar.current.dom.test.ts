// @vitest-environment jsdom
// A menu of the bar that stands for a part of the page (`current`): its title carries the dot and says what is shown.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it } from 'vitest';
import MenuBar from './MenuBar.svelte';
import type { Menu } from './menu';

const radio = (label: string, checked: boolean) => ({ kind: 'radio' as const, label, checked, onselect: () => {} });

let app: ReturnType<typeof mount> | undefined;
const start = (menus: Menu[], labels: { submenuCurrent?: string } = {}): void => {
  document.body.innerHTML = '';
  app = mount(MenuBar, { target: document.body, props: { menus, label: 'Menu', labels } });
  flushSync();
};
afterEach(async () => {
  if (app) await unmount(app);
  app = undefined;
});
const trigger = (id: string) => document.querySelector<HTMLButtonElement>(`button.mb-trigger[data-for="${id}"]`)!;

it('gives the title of a menu with a current part the dot and a name that says which', () => {
  start(
    [
      { id: 'view', label: 'View', items: [radio('Light', true)] },
      { id: 'learn', label: 'Learning', current: 'Paths', items: [radio('Paths', true)] },
    ],
    { submenuCurrent: '{label}, now: {current}' },
  );
  const learn = trigger('learn');
  expect(learn.getAttribute('aria-label')).toBe('Learning, now: Paths');
  expect(learn.querySelector('.mb-current')?.textContent).toBe('●');
  expect(learn.querySelector('.mb-current')?.getAttribute('aria-hidden')).toBe('true');
  // A checked radio item alone (a theme) is no current part: no dot, no name of its own.
  const view = trigger('view');
  expect(view.hasAttribute('aria-label')).toBe(false);
  expect(view.querySelector('.mb-current')).toBeNull();
});

it('takes the default words, and a menu without a current part has no dot', () => {
  start([
    { id: 'learn', label: 'Learning', current: 'Map', items: [radio('Map', true)] },
    { id: 'other', label: 'Other', current: null, items: [radio('Paths', false)] },
  ]);
  expect(trigger('learn').getAttribute('aria-label')).toBe('Learning, Map');
  expect(trigger('other').hasAttribute('aria-label')).toBe(false);
  expect(trigger('other').querySelector('.mb-current')).toBeNull();
  // The popup keeps the plain name of the menu.
  trigger('learn').click();
  flushSync();
  expect(document.querySelector('[role="menu"][data-menu="learn"]')?.getAttribute('aria-label')).toBe('Learning');
});
